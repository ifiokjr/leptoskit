// The live scenario for the generated Go client; see `sdk_live.rs`. It runs
// as a test next to the generated package.

package sdklive

import (
	"context"
	"fmt"
	"os"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"
)

// person is one person's client, holding the latest value of their
// subscription.
type person struct {
	token  string
	client *Client
	notes  *LiveSubscription[[]SdkNote]
	page   *LiveSubscription[SdkPage]

	mu       sync.Mutex
	statuses []LiveState
}

func newPerson(ctx context.Context, t *testing.T, serverURL string, prefix string, token string) *person {
	client := NewClient(serverURL)
	client.LiveOptions = LiveOptions{
		Token:      func(context.Context) (string, error) { return token, nil },
		MinBackoff: 50 * time.Millisecond,
		MaxBackoff: 200 * time.Millisecond,
	}
	p := &person{token: token, client: client}
	client.Live().OnStatus(func(status LiveStatus) {
		p.mu.Lock()
		defer p.mu.Unlock()
		p.statuses = append(p.statuses, status.State)
	})
	notes, err := client.SubscribeSdkNotes(ctx, prefix)
	if err != nil {
		t.Fatalf("%s subscribes: %v", token, err)
	}
	p.notes = notes
	page, err := client.SubscribeSdkPageRows(ctx, prefix)
	if err != nil {
		t.Fatalf("%s subscribes to the page: %v", token, err)
	}
	p.page = page
	go func() {
		for err := range notes.Errors() {
			fmt.Fprintf(os.Stderr, "%s: %v\n", token, err)
		}
	}()
	return p
}

// has counts the notes that say body.
func (p *person) has(body string) int {
	notes, _ := p.notes.Value()
	count := 0
	for _, note := range notes {
		if note.Body == body {
			count++
		}
	}
	return count
}

func (p *person) reconnected() bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, state := range p.statuses {
		if state == LiveStateReconnecting {
			return true
		}
	}
	return false
}

// until waits for test to hold.
func until(t *testing.T, what string, test func() bool) {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for !test() {
		if time.Now().After(deadline) {
			t.Fatalf("waited too long for %s", what)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func names(members []LiveMember[SdkPresence]) string {
	var out []string
	for _, member := range members {
		out = append(out, member.ID+":"+member.State.Name)
	}
	sort.Strings(out)
	return strings.Join(out, ",")
}

func TestLiveScenario(t *testing.T) {
	serverURL, prefix := os.Getenv("SDK_LIVE_URL"), os.Getenv("SDK_LIVE_PREFIX")
	if serverURL == "" || prefix == "" {
		t.Fatal("SDK_LIVE_URL and SDK_LIVE_PREFIX are not set")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	go func() {
		<-ctx.Done()
		if ctx.Err() == context.DeadlineExceeded {
			fmt.Fprintln(os.Stderr, "timed out")
			os.Exit(1)
		}
	}()

	ada := newPerson(ctx, t, serverURL, prefix, "ada")
	bob := newPerson(ctx, t, serverURL, prefix, "bob")
	defer ada.client.Live().Close()
	defer bob.client.Live().Close()

	for _, someone := range []*person{ada, bob} {
		until(t, someone.token+": the first value", func() bool {
			_, notesOK := someone.notes.Value()
			_, pageOK := someone.page.Value()
			return notesOK && pageOK
		})
	}
	if notes, _ := ada.notes.Value(); len(notes) != 0 {
		t.Fatalf("a fresh database has no notes: %v", notes)
	}
	fmt.Println("ok initial")

	// Read-your-writes: when the call returns, Bob's own subscription has it.
	first, err := bob.client.AddSdkNoteLive(ctx, prefix+"one")
	if err != nil || first == "" {
		t.Fatalf("the mutation returns the id: %q, %v", first, err)
	}
	if bob.has(prefix+"one") != 1 {
		notes, _ := bob.notes.Value()
		t.Fatalf("Bob sees his own write: %v", notes)
	}
	if page, ok := bob.page.Value(); !ok || len(page.Items) != 1 || page.End == nil || *page.End != first || !page.HasMore {
		t.Fatalf("Bob's page rows and metadata reflect his write: %+v", page)
	}
	fmt.Println("ok read-your-writes")

	// Ada's subscription receives it as a patch.
	until(t, "ada: Bob's note", func() bool { return ada.has(prefix+"one") == 1 })
	fmt.Println("ok patch")

	if _, err := bob.client.AddSdkNoteLive(ctx, prefix+"alpha"); err != nil {
		t.Fatal(err)
	}
	until(t, "an insertion before the existing row", func() bool {
		notes, _ := ada.notes.Value()
		page, _ := ada.page.Value()
		return len(notes) == 2 && len(page.Items) == 2
	})
	notes, _ := ada.notes.Value()
	page, _ := ada.page.Value()
	if ada.has(prefix+"one") != 1 || notes[0].Body != prefix+"alpha" {
		t.Fatal("inserting preserves the existing row")
	}
	if page.Items[0].Body != prefix+"alpha" || page.End == nil || *page.End != first || page.HasMore {
		t.Fatalf("page patches update rows and metadata together: %+v", page)
	}
	fmt.Println("ok insertion and page")

	// The proxy lets this through, then cuts the connection before the answer.
	dropped, err := ada.client.AddSdkNoteLive(ctx, prefix+"drop-me")
	if err != nil || dropped == "" {
		t.Fatalf("the re-sent mutation answers: %q, %v", dropped, err)
	}
	until(t, "ada: the dropped note", func() bool { return ada.has(prefix+"drop-me") == 1 })
	if !ada.reconnected() {
		t.Fatalf("Ada reconnected: %v", ada.statuses)
	}
	if ada.has(prefix+"drop-me") != 1 {
		t.Fatal("the re-sent mutation ran once")
	}
	fmt.Println("ok reconnect")

	last, err := bob.client.AddSdkNoteLive(ctx, prefix+"two")
	if err != nil {
		t.Fatal(err)
	}
	until(t, "ada: a note after the reconnect", func() bool { return ada.has(prefix+"two") == 1 })
	until(t, "the resumed page", func() bool {
		page, _ := ada.page.Value()
		return len(page.Items) == 4
	})
	notes, _ = ada.notes.Value()
	page, _ = ada.page.Value()
	if len(notes) != 4 || page.End == nil || *page.End != last || page.HasMore {
		t.Fatalf("resumed list and page preserve every row and update the end cursor: %+v", page)
	}
	fmt.Println("ok resumed")

	// Channels: presence from both, and Bob's signal reaches Ada.
	adaRoom, err := ada.client.JoinSdkRoom(ctx, SdkRoom{Room: prefix}, &SdkPresence{Name: "Ada"})
	if err != nil {
		t.Fatal(err)
	}
	go func() {
		for err := range adaRoom.Errors() {
			fmt.Fprintf(os.Stderr, "ada join: %v\n", err)
		}
	}()
	bobRoom, err := bob.client.JoinSdkRoom(ctx, SdkRoom{Room: prefix}, &SdkPresence{Name: "Bob"})
	if err != nil {
		t.Fatal(err)
	}
	until(t, "both in the room", func() bool {
		return names(adaRoom.Members()) == "ada:Ada,bob:Bob" && names(bobRoom.Members()) == "ada:Ada,bob:Bob"
	})
	if err := bobRoom.Signal(SdkSignal{Text: "hi"}); err != nil {
		t.Fatal(err)
	}
	select {
	case signal := <-adaRoom.Signals():
		if signal.Message.Text != "hi" || signal.From != "bob" {
			t.Fatalf("the signal: %+v", signal)
		}
	case <-time.After(20 * time.Second):
		t.Fatal("waited too long for Bob's signal")
	}
	adaRoom.Leave()
	bobRoom.Leave()
	fmt.Println("ok channel")

	ada.notes.Close()
	bob.notes.Close()
	ada.page.Close()
	bob.page.Close()
}
