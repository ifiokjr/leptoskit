package presence

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"reflect"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

func TestModelPresence(t *testing.T) {
	for _, wire := range []string{
		`{}`,
		`{"enabled":false,"count":0,"text":"","items":[],"labels":{},"flag":false}`,
		`{"enabled":true,"count":7,"text":"value","items":["one"],"labels":{"a":"b"},"flag":true}`,
	} {
		var model Payload
		if err := json.Unmarshal([]byte(wire), &model); err != nil {
			t.Fatal(err)
		}
		encoded, err := json.Marshal(model)
		if err != nil {
			t.Fatal(err)
		}
		var want, got map[string]any
		if err := json.Unmarshal([]byte(wire), &want); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(encoded, &got); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("presence changed: input %s, output %s", wire, encoded)
		}
	}
}

func TestRequestPresence(t *testing.T) {
	requests := make(chan map[string]any, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		got := map[string]any{}
		if r.URL.Path == "/send" {
			if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
				t.Error(err)
			}
		} else {
			if err := r.ParseForm(); err != nil {
				t.Error(err)
			}
			for key, values := range r.Form {
				got[key] = values[0]
			}
		}
		requests <- got
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{}`))
	}))
	defer server.Close()
	client := NewClient(server.URL)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	for _, call := range []struct {
		name string
		fn   func(context.Context, *bool, *int64, *string) (Payload, error)
	}{
		{"json", client.Send},
		{"form", client.Form},
		{"query", client.Query},
	} {
		if _, err := call.fn(ctx, nil, nil, nil); err != nil {
			t.Fatal(err)
		}
		if got := <-requests; len(got) != 0 {
			t.Fatalf("%s absent values were sent: %v", call.name, got)
		}
		enabled, count, text := false, int64(0), ""
		if _, err := call.fn(ctx, &enabled, &count, &text); err != nil {
			t.Fatal(err)
		}
		want := map[string]any{"enabled": "false", "count": "0", "text": ""}
		if call.name == "json" {
			want = map[string]any{"enabled": false, "count": float64(0), "text": ""}
		}
		if got := <-requests; !reflect.DeepEqual(got, want) {
			t.Fatalf("%s explicit zero values changed: got %v, want %v", call.name, got, want)
		}
	}
}

func TestLiveRequestPresence(t *testing.T) {
	requests := make(chan map[string]any, 2)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upgrader := websocket.Upgrader{}
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.Close()
		for {
			var frame map[string]json.RawMessage
			if err := conn.ReadJSON(&frame); err != nil {
				return
			}
			switch string(frame["type"]) {
			case `"hello"`:
				err = conn.WriteJSON(map[string]any{"type": "welcome", "version": 0})
			case `"mutate"`:
				var args map[string]any
				if err := json.Unmarshal(frame["args"], &args); err != nil {
					t.Error(err)
					return
				}
				requests <- args
				err = conn.WriteJSON(map[string]any{"type": "done", "request": frame["request"], "ok": map[string]any{}, "version": 0})
			}
			if err != nil {
				t.Error(err)
				return
			}
		}
	}))
	defer server.Close()
	client := NewClient(server.URL)
	defer client.Live().Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := client.SendLive(ctx, nil, nil, nil); err != nil {
		t.Fatal(err)
	}
	if got := <-requests; len(got) != 0 {
		t.Fatalf("absent values were sent over live: %v", got)
	}
	enabled, count, text := false, int64(0), ""
	if _, err := client.SendLive(ctx, &enabled, &count, &text); err != nil {
		t.Fatal(err)
	}
	want := map[string]any{"enabled": false, "count": float64(0), "text": ""}
	if got := <-requests; !reflect.DeepEqual(got, want) {
		t.Fatalf("live explicit zero values changed: got %v, want %v", got, want)
	}
}
