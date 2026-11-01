// The live protocol runtime: one WebSocket to the server's live path,
// carrying subscriptions, mutations and channels, kept up across reconnects.
// The generator writes the package clause above this file; its one
// dependency is github.com/gorilla/websocket.

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	mathrand "math/rand"
	"net/http"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

// LiveSubprotocol is the WebSocket subprotocol the server speaks.
const LiveSubprotocol = "leptoskit.live.v1.json"

// LiveError is a failure the server reported over the live connection.
type LiveError struct {
	// Status is the HTTP status the same failure would carry; 0 when the
	// connection closed or the client failed.
	Status int `json:"status"`
	// Message is the human-readable message.
	Message string `json:"message"`
}

// Error implements error.
func (e *LiveError) Error() string {
	return e.Message
}

func liveClosed() *LiveError {
	return &LiveError{Message: "the live connection closed"}
}

// LiveState is where the live connection is.
type LiveState string

const (
	LiveStateConnecting   LiveState = "connecting"
	LiveStateLive         LiveState = "live"
	LiveStateReconnecting LiveState = "reconnecting"
	LiveStateOffline      LiveState = "offline"
)

// LiveStatus is where the live connection is, for a "reconnecting…" banner.
type LiveStatus struct {
	State LiveState
	// Attempt counts the tries since the connection was last live, while
	// connecting or reconnecting.
	Attempt int
	// NextIn is how long until the next try, while reconnecting.
	NextIn time.Duration
	// Version is the version the server started from, while live.
	Version uint64
	// Connection is the server's name for the connection, while live.
	Connection string
}

// LiveMember is someone in a channel, and the presence they joined with.
type LiveMember[P any] struct {
	// ID is who: the caller id the server knows them by.
	ID string `json:"id"`
	// State is what they show the others.
	State P `json:"state"`
}

// LiveSignal is a message another member of a channel sent.
type LiveSignal[M any] struct {
	Message M
	// From is who sent it, when the server says.
	From string
}

// LiveOptions is how the live connection behaves.
type LiveOptions struct {
	// Token returns the bearer token the connection signs in with, such as a
	// native session's access token. It is called on every connect, and again
	// when the server says the token is about to expire.
	Token func(ctx context.Context) (string, error)
	// OnAuthRefused is called when the server refuses the token.
	OnAuthRefused func()
	// Header is sent with the WebSocket upgrade request, such as a Cookie.
	Header http.Header
	// Dialer opens the socket; a dialer with the environment's proxy when nil.
	Dialer *websocket.Dialer
	// ReadYourWrites is how long a live mutation waits for its own write to
	// reach the subscriptions before it returns anyway. Defaults to 2 seconds.
	ReadYourWrites time.Duration
	// MinBackoff is the first reconnect delay's upper bound. Defaults to 250
	// milliseconds.
	MinBackoff time.Duration
	// MaxBackoff is the longest reconnect delay. Defaults to 30 seconds.
	MaxBackoff time.Duration
}

// LiveURL returns the live socket's URL for a server origin such as
// https://api.example.com.
func LiveURL(serverURL string, path string) string {
	serverURL = strings.TrimRight(serverURL, "/")
	if strings.HasPrefix(serverURL, "http") {
		serverURL = "ws" + strings.TrimPrefix(serverURL, "http")
	}
	return serverURL + path
}

// LiveChannelKey returns the name of a channel instance: its join arguments
// as compact JSON, with the fields in the order the server declares them, as
// the app's own client writes it, so every client of the same instance meets
// in one room.
func LiveChannelKey(fields []string, args any) (string, error) {
	encoded, err := liveEncode(args)
	if err != nil {
		return "", err
	}
	keys, values, err := liveObject(encoded)
	if err != nil {
		return "", err
	}
	var out bytes.Buffer
	written := map[string]bool{}
	write := func(key string, value json.RawMessage) {
		if len(written) > 0 {
			out.WriteByte(',')
		}
		written[key] = true
		name, _ := liveEncode(key)
		out.Write(name)
		out.WriteByte(':')
		out.Write(value)
	}
	out.WriteByte('{')
	for _, field := range fields {
		if written[field] {
			continue
		}
		value, ok := values[field]
		if !ok {
			value = json.RawMessage("null")
		}
		write(field, value)
	}
	for _, key := range keys {
		if !written[key] {
			write(key, values[key])
		}
	}
	out.WriteByte('}')
	return out.String(), nil
}

// liveEncode writes value as compact JSON the way serde_json does: no HTML
// escapes, and U+2028 and U+2029 as themselves.
func liveEncode(value any) ([]byte, error) {
	var out bytes.Buffer
	encoder := json.NewEncoder(&out)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		return nil, err
	}
	encoded := bytes.TrimSuffix(out.Bytes(), []byte("\n"))
	if !bytes.Contains(encoded, []byte(`\u202`)) {
		return encoded, nil
	}
	fixed := make([]byte, 0, len(encoded))
	inString := false
	for index := 0; index < len(encoded); index++ {
		character := encoded[index]
		switch {
		case !inString:
			inString = character == '"'
		case character == '"':
			inString = false
		case character == '\\' && index+1 < len(encoded):
			if index+6 <= len(encoded) {
				switch string(encoded[index : index+6]) {
				case `\u2028`:
					fixed = append(fixed, "\u2028"...)
					index += 5
					continue
				case `\u2029`:
					fixed = append(fixed, "\u2029"...)
					index += 5
					continue
				}
			}
			fixed = append(fixed, encoded[index:index+2]...)
			index++
			continue
		}
		fixed = append(fixed, character)
	}
	return fixed, nil
}

// liveObject splits a JSON object into its keys, in order, and their raw
// values; anything else has neither.
func liveObject(encoded []byte) ([]string, map[string]json.RawMessage, error) {
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	token, err := decoder.Token()
	if err != nil {
		return nil, nil, err
	}
	if delim, ok := token.(json.Delim); !ok || delim != '{' {
		return nil, nil, nil
	}
	var keys []string
	values := map[string]json.RawMessage{}
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return nil, nil, err
		}
		key, _ := token.(string)
		var value json.RawMessage
		if err := decoder.Decode(&value); err != nil {
			return nil, nil, err
		}
		if _, seen := values[key]; !seen {
			keys = append(keys, key)
		}
		values[key] = value
	}
	return keys, values, nil
}

// livePresent encodes call arguments without their absent optional fields,
// as the HTTP calls send them.
func livePresent(args any) (json.RawMessage, error) {
	if args == nil {
		return json.RawMessage("{}"), nil
	}
	encoded, err := liveEncode(args)
	if err != nil {
		return nil, &LiveError{Message: err.Error()}
	}
	keys, values, err := liveObject(encoded)
	if err != nil {
		return nil, &LiveError{Message: err.Error()}
	}
	if values == nil {
		if string(encoded) == "null" {
			return json.RawMessage("{}"), nil
		}
		return encoded, nil
	}
	var out bytes.Buffer
	out.WriteByte('{')
	for _, key := range keys {
		if string(values[key]) == "null" {
			continue
		}
		if out.Len() > 1 {
			out.WriteByte(',')
		}
		name, _ := liveEncode(key)
		out.Write(name)
		out.WriteByte(':')
		out.Write(values[key])
	}
	out.WriteByte('}')
	return out.Bytes(), nil
}

// liveRandomKey returns a fresh idempotency key, a random UUID.
func liveRandomKey() string {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return fmt.Sprintf("%x-%x", time.Now().UnixNano(), mathrand.Int63())
	}
	raw[6] = raw[6]&0x0f | 0x40
	raw[8] = raw[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", raw[0:4], raw[4:6], raw[6:8], raw[8:10], raw[10:])
}

// liveSub is one subscription the connection holds.
type liveSub struct {
	query   string
	args    json.RawMessage
	value   json.RawMessage
	onValue func(json.RawMessage, uint64)
	onError func(*LiveError)
	onClose func()
}

// livePending is a mutation waiting for its answer.
type livePending struct {
	frame   []byte
	resolve func(json.RawMessage, error)
}

// liveWaiter is an answered mutation waiting for its write to reach the
// subscriptions.
type liveWaiter struct {
	version uint64
	value   json.RawMessage
	resolve func(json.RawMessage, error)
	timer   *time.Timer
}

// liveJoined is one channel the connection is in.
type liveJoined struct {
	name       string
	args       json.RawMessage
	state      json.RawMessage
	onSignal   func(json.RawMessage, string)
	onPresence func(json.RawMessage)
	onError    func(*LiveError)
	onClose    func()
}

// liveSession is one socket, from the dial until it closes.
type liveSession struct {
	ctx    context.Context
	cancel context.CancelFunc
	conn   *websocket.Conn
	// queue holds the frames waiting for the writer, under the connection's
	// lock.
	queue     [][]byte
	wake      chan struct{}
	lastHeard time.Time
	beat      context.CancelFunc
}

// LiveConnection is one live connection, opened on first use and kept up
// until Close. It is safe for concurrent use.
type LiveConnection struct {
	url     string
	options LiveOptions

	mu             sync.Mutex
	session        *liveSession
	ready          bool
	closed         bool
	status         LiveStatus
	listeners      map[int]func(LiveStatus)
	nextListener   int
	nextID         uint32
	subs           map[uint32]*liveSub
	pending        map[uint32]*livePending
	waiters        []*liveWaiter
	channels       map[string]*liveJoined
	version        uint64
	attempt        int
	retry          *time.Timer
	reconnectAfter *time.Duration
	// after holds the callbacks to run once the lock is released, in order.
	after      []func()
	dispatchMu sync.Mutex
}

// NewLiveConnection returns a connection to the live socket at url, such as
// wss://api.example.com/live. It connects on first use.
func NewLiveConnection(url string, options LiveOptions) *LiveConnection {
	return &LiveConnection{
		url:       url,
		options:   options,
		status:    LiveStatus{State: LiveStateOffline},
		listeners: map[int]func(LiveStatus){},
		subs:      map[uint32]*liveSub{},
		pending:   map[uint32]*livePending{},
		channels:  map[string]*liveJoined{},
	}
}

// Status returns where the connection is now.
func (l *LiveConnection) Status() LiveStatus {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.status
}

// Version returns the last version the connection applied.
func (l *LiveConnection) Version() uint64 {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.version
}

// OnStatus calls listener with every status change, until the returned
// function is called.
func (l *LiveConnection) OnStatus(listener func(LiveStatus)) func() {
	l.mu.Lock()
	defer l.mu.Unlock()
	id := l.nextListener
	l.nextListener++
	l.listeners[id] = listener
	return func() {
		l.mu.Lock()
		defer l.mu.Unlock()
		delete(l.listeners, id)
	}
}

// Reauthenticate sends the token again, such as after signing in.
func (l *LiveConnection) Reauthenticate(ctx context.Context) error {
	if l.options.Token == nil {
		return nil
	}
	token, err := l.options.Token(ctx)
	if err != nil {
		return err
	}
	if token != "" {
		l.mu.Lock()
		l.sendLocked(map[string]any{"type": "auth", "token": token})
		l.mu.Unlock()
	}
	return nil
}

// Close closes the connection for good: pending mutations fail, and every
// subscription's and channel's channels close.
func (l *LiveConnection) Close() error {
	defer l.flush()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		return nil
	}
	l.closed = true
	if l.retry != nil {
		l.retry.Stop()
		l.retry = nil
	}
	l.endLocked()
	for id, pending := range l.pending {
		resolve := pending.resolve
		l.later(func() { resolve(nil, liveClosed()) })
		delete(l.pending, id)
	}
	for _, waiter := range l.waiters {
		waiter.timer.Stop()
		resolve, value := waiter.resolve, waiter.value
		l.later(func() { resolve(value, nil) })
	}
	l.waiters = nil
	for id, sub := range l.subs {
		l.later(sub.onClose)
		delete(l.subs, id)
	}
	for key, joined := range l.channels {
		l.later(joined.onClose)
		delete(l.channels, key)
	}
	l.setStatusLocked(LiveStatus{State: LiveStateOffline})
	return nil
}

// later queues fn to run, in order, once the lock is released.
func (l *LiveConnection) later(fn func()) {
	l.after = append(l.after, fn)
}

// flush runs the queued callbacks, one goroutine at a time and in the order
// they were queued; a callback that calls back in finds its own work run
// after it returns.
func (l *LiveConnection) flush() {
	for {
		if !l.dispatchMu.TryLock() {
			return
		}
		for {
			l.mu.Lock()
			work := l.after
			l.after = nil
			l.mu.Unlock()
			if len(work) == 0 {
				break
			}
			for _, fn := range work {
				fn()
			}
		}
		l.dispatchMu.Unlock()
		l.mu.Lock()
		more := len(l.after) > 0
		l.mu.Unlock()
		if !more {
			return
		}
	}
}

func (l *LiveConnection) setStatusLocked(status LiveStatus) {
	l.status = status
	for _, listener := range l.listeners {
		listener := listener
		l.later(func() { listener(status) })
	}
}

func (l *LiveConnection) nextIDLocked() uint32 {
	l.nextID++
	if l.nextID == 0 {
		l.nextID++
	}
	return l.nextID
}

func (l *LiveConnection) busyLocked() bool {
	return len(l.subs) > 0 || len(l.pending) > 0 || len(l.channels) > 0
}

// ensureLocked opens the socket when nothing is open or waiting to retry.
func (l *LiveConnection) ensureLocked() {
	if l.session == nil && l.retry == nil && !l.closed {
		l.openLocked()
	}
}

// sendLocked sends frame now, if the connection is up; otherwise the next
// connect sends what the connection holds.
func (l *LiveConnection) sendLocked(frame any) {
	if !l.ready || l.session == nil {
		return
	}
	encoded, err := json.Marshal(frame)
	if err != nil {
		return
	}
	l.enqueueLocked(l.session, encoded)
}

func (l *LiveConnection) enqueueLocked(session *liveSession, frame []byte) {
	session.queue = append(session.queue, frame)
	select {
	case session.wake <- struct{}{}:
	default:
	}
}

func (l *LiveConnection) openLocked() {
	l.setStatusLocked(LiveStatus{State: LiveStateConnecting, Attempt: l.attempt})
	ctx, cancel := context.WithCancel(context.Background())
	session := &liveSession{ctx: ctx, cancel: cancel, wake: make(chan struct{}, 1)}
	l.session = session
	l.ready = false
	go l.run(session)
}

// endLocked closes the current socket, if there is one.
func (l *LiveConnection) endLocked() {
	session := l.session
	l.session = nil
	l.ready = false
	if session == nil {
		return
	}
	session.cancel()
	if session.conn != nil {
		_ = session.conn.Close()
	}
}

// run dials, says hello, and reads until the socket closes.
func (l *LiveConnection) run(session *liveSession) {
	dialer := websocket.Dialer{Proxy: http.ProxyFromEnvironment, HandshakeTimeout: 45 * time.Second}
	if l.options.Dialer != nil {
		dialer = *l.options.Dialer
	}
	dialer.Subprotocols = []string{LiveSubprotocol}
	conn, _, err := dialer.DialContext(session.ctx, l.url, l.options.Header)
	if err != nil {
		l.lost(session)
		return
	}
	l.mu.Lock()
	if l.session != session {
		l.mu.Unlock()
		_ = conn.Close()
		return
	}
	session.conn = conn
	session.lastHeard = time.Now()
	l.mu.Unlock()
	// The server pings every 30 seconds; the default handler answers.
	pong := conn.PingHandler()
	conn.SetPingHandler(func(data string) error {
		l.mu.Lock()
		session.lastHeard = time.Now()
		l.mu.Unlock()
		return pong(data)
	})
	go l.write(session)
	l.greet(session)
	for {
		_, data, err := conn.ReadMessage()
		if err != nil {
			l.lost(session)
			return
		}
		l.receive(session, data)
	}
}

// write sends the session's queued frames, one at a time.
func (l *LiveConnection) write(session *liveSession) {
	for {
		select {
		case <-session.wake:
		case <-session.ctx.Done():
			return
		}
		l.mu.Lock()
		frames := session.queue
		session.queue = nil
		l.mu.Unlock()
		for _, frame := range frames {
			_ = session.conn.SetWriteDeadline(time.Now().Add(10 * time.Second))
			if err := session.conn.WriteMessage(websocket.TextMessage, frame); err != nil {
				_ = session.conn.Close()
				return
			}
		}
	}
}

// greet says hello, signs in, and sends everything the connection holds.
func (l *LiveConnection) greet(session *liveSession) {
	l.mu.Lock()
	if l.session != session {
		l.mu.Unlock()
		return
	}
	hello := map[string]any{"type": "hello"}
	if l.version > 0 {
		hello["resume"] = map[string]uint64{"version": l.version}
	}
	encoded, _ := json.Marshal(hello)
	l.enqueueLocked(session, encoded)
	l.mu.Unlock()

	token := ""
	if l.options.Token != nil {
		ctx, cancel := context.WithTimeout(session.ctx, 30*time.Second)
		if found, err := l.options.Token(ctx); err == nil {
			token = found
		}
		cancel()
	}

	defer l.flush()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.session != session {
		return
	}
	raw := func(frame any) {
		if encoded, err := json.Marshal(frame); err == nil {
			l.enqueueLocked(session, encoded)
		}
	}
	if token != "" {
		raw(map[string]any{"type": "auth", "token": token})
	}
	ids := make([]uint32, 0, len(l.subs))
	for id := range l.subs {
		ids = append(ids, id)
	}
	sort.Slice(ids, func(a, b int) bool { return ids[a] < ids[b] })
	for _, id := range ids {
		sub := l.subs[id]
		raw(map[string]any{"type": "sub", "id": id, "query": sub.query, "args": sub.args})
	}
	keys := make([]string, 0, len(l.channels))
	for key := range l.channels {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		raw(liveJoinFrame(key, l.channels[key]))
	}
	// The same request id and key: the server's receipt answers a call that
	// already ran with its first result.
	ids = ids[:0]
	for id := range l.pending {
		ids = append(ids, id)
	}
	sort.Slice(ids, func(a, b int) bool { return ids[a] < ids[b] })
	for _, id := range ids {
		l.enqueueLocked(session, l.pending[id].frame)
	}
	l.ready = true
}

func liveJoinFrame(key string, joined *liveJoined) map[string]any {
	return map[string]any{
		"type":    "join",
		"channel": key,
		"name":    joined.name,
		"args":    joined.args,
		"state":   joined.state,
	}
}

// liveInbound is every field a server frame may carry.
type liveInbound struct {
	Type             string          `json:"type"`
	Connection       string          `json:"connection"`
	Version          *uint64         `json:"version"`
	HeartbeatSeconds uint64          `json:"heartbeat_seconds"`
	To               *uint64         `json:"to"`
	Updates          []liveUpdate    `json:"updates"`
	Request          uint32          `json:"request"`
	Ok               json.RawMessage `json:"ok"`
	Err              json.RawMessage `json:"err"`
	Refused          bool            `json:"refused"`
	Expiring         bool            `json:"expiring"`
	AfterMs          uint64          `json:"after_ms"`
	Channel          string          `json:"channel"`
	From             string          `json:"from"`
	Message          json.RawMessage `json:"message"`
	Members          json.RawMessage `json:"members"`
}

type liveUpdate struct {
	ID    uint32          `json:"id"`
	Ok    json.RawMessage `json:"ok"`
	Patch json.RawMessage `json:"patch"`
	Err   json.RawMessage `json:"err"`
}

func liveFailed(err json.RawMessage) bool {
	return len(err) > 0 && string(err) != "null"
}

func liveWireError(err json.RawMessage) *LiveError {
	wire := LiveError{Status: 500, Message: "the server failed"}
	_ = json.Unmarshal(err, &wire)
	return &wire
}

func liveValue(ok json.RawMessage) json.RawMessage {
	if len(ok) == 0 {
		return json.RawMessage("null")
	}
	return ok
}

func (l *LiveConnection) receive(session *liveSession, data []byte) {
	var frame liveInbound
	if err := json.Unmarshal(data, &frame); err != nil {
		// A field of a type this runtime doesn't expect leaves the rest
		// decoded; anything else isn't a frame.
		var typeError *json.UnmarshalTypeError
		if !errors.As(err, &typeError) {
			return
		}
	}
	defer l.flush()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.session != session {
		return
	}
	session.lastHeard = time.Now()
	switch frame.Type {
	case "welcome":
		l.attempt = 0
		if frame.Version != nil {
			l.version = *frame.Version
		}
		seconds := frame.HeartbeatSeconds
		if seconds == 0 {
			seconds = 30
		}
		l.heartbeatLocked(session, time.Duration(seconds)*time.Second)
		l.setStatusLocked(LiveStatus{State: LiveStateLive, Version: l.version, Connection: frame.Connection})
	case "transition":
		l.transitionLocked(&frame)
	case "done":
		l.doneLocked(&frame)
	case "auth_status":
		if frame.Refused {
			if refused := l.options.OnAuthRefused; refused != nil {
				l.later(refused)
			}
		} else if frame.Expiring {
			go func() { _ = l.Reauthenticate(context.Background()) }()
		}
	case "reconnect":
		after := time.Duration(frame.AfterMs) * time.Millisecond
		l.reconnectAfter = &after
	case "signal":
		if joined := l.channels[frame.Channel]; joined != nil {
			message, from := liveValue(frame.Message), frame.From
			l.later(func() { joined.onSignal(message, from) })
		}
	case "presence":
		if joined := l.channels[frame.Channel]; joined != nil && bytes.HasPrefix(bytes.TrimSpace(frame.Members), []byte("[")) {
			members := frame.Members
			l.later(func() { joined.onPresence(members) })
		}
	case "channel_error":
		if joined := l.channels[frame.Channel]; joined != nil {
			err := liveWireError(frame.Err)
			l.later(func() { joined.onError(err) })
		}
	default:
		// pong, and frames a newer server sends that this runtime doesn't
		// know.
	}
}

// transitionLocked applies one transition: every update, then every
// listener, then the version.
func (l *LiveConnection) transitionLocked(frame *liveInbound) {
	type change struct {
		sub   *liveSub
		value json.RawMessage
	}
	var changed []change
	var failed []func()
	values := map[*liveSub]json.RawMessage{}
	for _, update := range frame.Updates {
		sub := l.subs[update.ID]
		if sub == nil {
			continue
		}
		current, seen := values[sub]
		if !seen {
			current = sub.value
		}
		switch {
		case len(update.Ok) > 0:
			values[sub] = update.Ok
			changed = append(changed, change{sub, update.Ok})
		case len(update.Patch) > 0:
			patched := liveApplyPatch(current, update.Patch)
			values[sub] = patched
			changed = append(changed, change{sub, patched})
		case liveFailed(update.Err):
			err, onError := liveWireError(update.Err), sub.onError
			failed = append(failed, func() { onError(err) })
		}
	}
	to := l.version
	if frame.To != nil {
		to = *frame.To
	}
	if to > l.version {
		l.version = to
	}
	for sub, value := range values {
		sub.value = value
	}
	for _, item := range changed {
		onValue, value := item.sub.onValue, item.value
		l.later(func() { onValue(value, to) })
	}
	for _, fn := range failed {
		l.later(fn)
	}
	l.reachedLocked(to)
}

func (l *LiveConnection) doneLocked(frame *liveInbound) {
	var version uint64
	if frame.Version != nil {
		version = *frame.Version
	}
	if pending := l.pending[frame.Request]; pending != nil {
		delete(l.pending, frame.Request)
		resolve := pending.resolve
		if liveFailed(frame.Err) {
			err := liveWireError(frame.Err)
			l.later(func() { resolve(nil, err) })
			return
		}
		value := liveValue(frame.Ok)
		if len(l.subs) == 0 || l.version >= version {
			l.later(func() { resolve(value, nil) })
			return
		}
		waiter := &liveWaiter{version: version, value: value, resolve: resolve}
		wait := l.options.ReadYourWrites
		if wait <= 0 {
			wait = 2 * time.Second
		}
		waiter.timer = time.AfterFunc(wait, func() {
			defer l.flush()
			l.mu.Lock()
			defer l.mu.Unlock()
			for index, other := range l.waiters {
				if other == waiter {
					l.waiters = append(l.waiters[:index], l.waiters[index+1:]...)
					l.later(func() { resolve(value, nil) })
					return
				}
			}
		})
		l.waiters = append(l.waiters, waiter)
		return
	}
	sub := l.subs[frame.Request]
	if sub == nil {
		return
	}
	if liveFailed(frame.Err) {
		err, onError := liveWireError(frame.Err), sub.onError
		l.later(func() { onError(err) })
		return
	}
	value, onValue := liveValue(frame.Ok), sub.onValue
	sub.value = value
	l.later(func() { onValue(value, version) })
	// A fresh first value reflects every write the server had committed.
	l.reachedLocked(version)
}

// reachedLocked resolves the mutations whose writes the subscriptions now
// show.
func (l *LiveConnection) reachedLocked(version uint64) {
	waiting := l.waiters[:0]
	for _, waiter := range l.waiters {
		if waiter.version > version {
			waiting = append(waiting, waiter)
			continue
		}
		waiter.timer.Stop()
		resolve, value := waiter.resolve, waiter.value
		l.later(func() { resolve(value, nil) })
	}
	l.waiters = waiting
}

// heartbeatLocked pings every interval, and closes a socket that has been
// silent for two missed heartbeats.
func (l *LiveConnection) heartbeatLocked(session *liveSession, interval time.Duration) {
	if session.beat != nil {
		session.beat()
	}
	ctx, cancel := context.WithCancel(session.ctx)
	session.beat = cancel
	go func() {
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
			}
			l.mu.Lock()
			if l.session != session {
				l.mu.Unlock()
				return
			}
			if time.Since(session.lastHeard) > 2*interval+5*time.Second {
				// The connection is gone without saying so; the reader
				// sees it close.
				l.mu.Unlock()
				_ = session.conn.Close()
				return
			}
			l.sendLocked(map[string]string{"type": "ping"})
			l.mu.Unlock()
		}
	}()
}

// lost handles a socket that closed: try again after a backoff, or when the
// server said.
func (l *LiveConnection) lost(session *liveSession) {
	defer l.flush()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.session != session {
		return
	}
	l.endLocked()
	if l.closed {
		return
	}
	if !l.busyLocked() {
		l.reconnectAfter = nil
		l.setStatusLocked(LiveStatus{State: LiveStateOffline})
		return
	}
	var delay time.Duration
	if l.reconnectAfter != nil {
		delay = *l.reconnectAfter
		l.reconnectAfter = nil
	} else {
		low, high := l.options.MinBackoff, l.options.MaxBackoff
		if low <= 0 {
			low = 250 * time.Millisecond
		}
		if high <= 0 {
			high = 30 * time.Second
		}
		bound := low
		for step := 0; step < l.attempt && bound < high; step++ {
			bound *= 2
		}
		if bound > high {
			bound = high
		}
		if bound > 0 {
			delay = time.Duration(mathrand.Int63n(int64(bound)))
		}
	}
	l.attempt++
	l.setStatusLocked(LiveStatus{State: LiveStateReconnecting, Attempt: l.attempt, NextIn: delay})
	l.retry = time.AfterFunc(delay, func() {
		defer l.flush()
		l.mu.Lock()
		defer l.mu.Unlock()
		l.retry = nil
		if !l.closed && l.session == nil {
			l.openLocked()
		}
	})
}

// liveApplyPatch applies a keyed patch: removals first, then each upsert at
// its index.
func liveApplyPatch(value json.RawMessage, patch json.RawMessage) json.RawMessage {
	if !bytes.HasPrefix(bytes.TrimSpace(value), []byte("[")) {
		return value
	}
	var items []json.RawMessage
	if err := json.Unmarshal(value, &items); err != nil {
		return value
	}
	var changes struct {
		Remove []string `json:"remove"`
		Upsert []struct {
			At    *int            `json:"at"`
			Value json.RawMessage `json:"value"`
		} `json:"upsert"`
	}
	if err := json.Unmarshal(patch, &changes); err != nil {
		return value
	}
	remove := map[string]bool{}
	for _, id := range changes.Remove {
		remove[id] = true
	}
	kept := items[:0]
	for _, item := range items {
		var row struct {
			ID any `json:"id"`
		}
		if json.Unmarshal(item, &row) == nil {
			if id, ok := row.ID.(string); ok && remove[id] {
				continue
			}
		}
		kept = append(kept, item)
	}
	for _, upsert := range changes.Upsert {
		at := len(kept)
		if upsert.At != nil {
			at = *upsert.At
		}
		row := liveValue(upsert.Value)
		if at >= 0 && at < len(kept) {
			kept[at] = row
		} else {
			kept = append(kept, row)
		}
	}
	var out bytes.Buffer
	out.WriteByte('[')
	for index, item := range kept {
		if index > 0 {
			out.WriteByte(',')
		}
		out.Write(item)
	}
	out.WriteByte(']')
	return out.Bytes()
}

// liveQueue hands values to a channel without ever blocking the connection:
// a queue keeps every value until it is read, and a latest-only queue keeps
// just the newest.
type liveQueue[T any] struct {
	mu         sync.Mutex
	items      []T
	generation uint64
	latest     bool
	wake       chan struct{}
	out        chan T
	done       chan struct{}
	once       sync.Once
}

func newLiveQueue[T any](latest bool) *liveQueue[T] {
	queue := &liveQueue[T]{
		latest: latest,
		wake:   make(chan struct{}, 1),
		out:    make(chan T),
		done:   make(chan struct{}),
	}
	go queue.run()
	return queue
}

func (q *liveQueue[T]) push(item T) {
	q.mu.Lock()
	if q.latest {
		q.items = append(q.items[:0], item)
	} else {
		q.items = append(q.items, item)
	}
	q.generation++
	q.mu.Unlock()
	select {
	case q.wake <- struct{}{}:
	default:
	}
}

func (q *liveQueue[T]) run() {
	defer close(q.out)
	for {
		q.mu.Lock()
		if len(q.items) == 0 {
			q.mu.Unlock()
			select {
			case <-q.wake:
				continue
			case <-q.done:
				return
			}
		}
		item, generation := q.items[0], q.generation
		q.mu.Unlock()
		select {
		case q.out <- item:
			q.mu.Lock()
			// A newer value replaced a latest-only queue's while it waited.
			if !q.latest || q.generation == generation {
				q.items = q.items[1:]
			}
			q.mu.Unlock()
		case <-q.wake:
		case <-q.done:
			return
		}
	}
}

func (q *liveQueue[T]) close() {
	q.once.Do(func() { close(q.done) })
}

// LiveSubscription is a query the live connection watches.
type LiveSubscription[T any] struct {
	values  *liveQueue[T]
	errors  *liveQueue[*LiveError]
	mu      sync.Mutex
	value   T
	version uint64
	has     bool
	stop    func()
	once    sync.Once
	done    chan struct{}
}

// Values returns the first value and each new one. A reader that falls
// behind skips to the newest. It closes when the subscription does.
func (s *LiveSubscription[T]) Values() <-chan T {
	return s.values.out
}

// Errors returns the query's failures; the subscription stays, and recovers
// on the next change.
func (s *LiveSubscription[T]) Errors() <-chan *LiveError {
	return s.errors.out
}

// Value returns the latest value, and whether one has arrived yet.
func (s *LiveSubscription[T]) Value() (T, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.value, s.has
}

// Version returns the version the latest value reflects.
func (s *LiveSubscription[T]) Version() uint64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.version
}

// Close stops the subscription and closes its channels.
func (s *LiveSubscription[T]) Close() {
	if s.stop != nil {
		s.stop()
	}
	s.shut()
}

func (s *LiveSubscription[T]) shut() {
	s.once.Do(func() {
		close(s.done)
		s.values.close()
		s.errors.close()
	})
}

// LiveSubscribe watches the query name over l: the subscription gets the
// first value and each new one until Close, or until ctx is done.
func LiveSubscribe[T any](ctx context.Context, l *LiveConnection, name string, args any) (*LiveSubscription[T], error) {
	payload, err := livePresent(args)
	if err != nil {
		return nil, err
	}
	s := &LiveSubscription[T]{
		values: newLiveQueue[T](true),
		errors: newLiveQueue[*LiveError](false),
		done:   make(chan struct{}),
	}
	sub := &liveSub{
		query: name,
		args:  payload,
		onValue: func(raw json.RawMessage, version uint64) {
			var value T
			if err := json.Unmarshal(raw, &value); err != nil {
				s.errors.push(&LiveError{Message: fmt.Sprintf("the value was not the expected JSON: %v", err)})
				return
			}
			s.mu.Lock()
			s.value, s.version, s.has = value, version, true
			s.mu.Unlock()
			s.values.push(value)
		},
		onError: s.errors.push,
		onClose: s.shut,
	}
	defer l.flush()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		s.shut()
		return nil, liveClosed()
	}
	id := l.nextIDLocked()
	l.subs[id] = sub
	l.sendLocked(map[string]any{"type": "sub", "id": id, "query": name, "args": payload})
	l.ensureLocked()
	s.stop = func() {
		l.mu.Lock()
		defer l.mu.Unlock()
		if l.subs[id] == sub {
			delete(l.subs, id)
			l.sendLocked(map[string]any{"type": "unsub", "id": id})
		}
	}
	go func() {
		select {
		case <-ctx.Done():
			s.Close()
		case <-s.done:
		}
	}()
	return s, nil
}

// LiveMutate runs the mutation name over l. It returns once l's
// subscriptions show the write (read-your-writes), or after
// LiveOptions.ReadYourWrites if no change to them arrives. A call made while
// the connection is down waits for it, and one whose answer is lost is sent
// again with the same request id and key, so it runs once. The key is the
// one WithIdempotencyKey put on ctx, or a fresh one.
func LiveMutate[T any](ctx context.Context, l *LiveConnection, name string, args any) (T, error) {
	var zero T
	payload, err := livePresent(args)
	if err != nil {
		return zero, err
	}
	key, _ := ctx.Value(idempotencyKeyContext{}).(string)
	if key == "" {
		key = liveRandomKey()
	}
	type answer struct {
		value json.RawMessage
		err   error
	}
	answered := make(chan answer, 1)
	l.mu.Lock()
	if l.closed {
		l.mu.Unlock()
		return zero, liveClosed()
	}
	id := l.nextIDLocked()
	frame, err := json.Marshal(map[string]any{"type": "mutate", "request": id, "name": name, "args": payload, "key": key})
	if err != nil {
		l.mu.Unlock()
		return zero, &LiveError{Message: err.Error()}
	}
	l.pending[id] = &livePending{
		frame:   frame,
		resolve: func(value json.RawMessage, err error) { answered <- answer{value, err} },
	}
	if l.ready && l.session != nil {
		l.enqueueLocked(l.session, frame)
	}
	l.ensureLocked()
	l.mu.Unlock()
	l.flush()

	select {
	case got := <-answered:
		if got.err != nil {
			return zero, got.err
		}
		var result T
		if string(got.value) != "null" {
			if err := json.Unmarshal(got.value, &result); err != nil {
				return zero, &LiveError{Message: fmt.Sprintf("the result was not the expected JSON: %v", err)}
			}
		}
		return result, nil
	case <-ctx.Done():
		l.mu.Lock()
		delete(l.pending, id)
		l.mu.Unlock()
		return zero, ctx.Err()
	}
}

// LiveChannel is a joined channel instance.
type LiveChannel[M any, P any] struct {
	l        *LiveConnection
	key      string
	joined   *liveJoined
	signals  *liveQueue[LiveSignal[M]]
	presence *liveQueue[[]LiveMember[P]]
	errors   *liveQueue[*LiveError]
	mu       sync.Mutex
	members  []LiveMember[P]
	once     sync.Once
	done     chan struct{}
}

// Key returns the instance's name, from its join arguments.
func (c *LiveChannel[M, P]) Key() string {
	return c.key
}

// Signals returns each signal another member sends, and who sent it.
func (c *LiveChannel[M, P]) Signals() <-chan LiveSignal[M] {
	return c.signals.out
}

// Presence returns everyone in the channel, after each join and leave. A
// reader that falls behind skips to the newest.
func (c *LiveChannel[M, P]) Presence() <-chan []LiveMember[P] {
	return c.presence.out
}

// Errors returns the server's refusals, such as of the join.
func (c *LiveChannel[M, P]) Errors() <-chan *LiveError {
	return c.errors.out
}

// Members returns everyone in the channel, as last reported.
func (c *LiveChannel[M, P]) Members() []LiveMember[P] {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]LiveMember[P](nil), c.members...)
}

// Signal sends message to everyone else in the channel, at most once.
func (c *LiveChannel[M, P]) Signal(message M) error {
	encoded, err := json.Marshal(message)
	if err != nil {
		return &LiveError{Message: err.Error()}
	}
	c.l.mu.Lock()
	defer c.l.mu.Unlock()
	if c.l.channels[c.key] != c.joined {
		return &LiveError{Message: "the channel was left"}
	}
	c.l.sendLocked(map[string]any{"type": "signal", "channel": c.key, "message": json.RawMessage(encoded)})
	return nil
}

// SetPresence changes what this client shows the others.
func (c *LiveChannel[M, P]) SetPresence(state P) error {
	encoded, err := json.Marshal(state)
	if err != nil {
		return &LiveError{Message: err.Error()}
	}
	c.l.mu.Lock()
	defer c.l.mu.Unlock()
	if c.l.channels[c.key] != c.joined {
		return &LiveError{Message: "the channel was left"}
	}
	c.joined.state = encoded
	c.l.sendLocked(liveJoinFrame(c.key, c.joined))
	return nil
}

// Leave leaves the channel and closes its channels.
func (c *LiveChannel[M, P]) Leave() {
	c.l.mu.Lock()
	if c.l.channels[c.key] == c.joined {
		delete(c.l.channels, c.key)
		c.l.sendLocked(map[string]string{"type": "leave", "channel": c.key})
	}
	c.l.mu.Unlock()
	c.shut()
}

func (c *LiveChannel[M, P]) shut() {
	c.once.Do(func() {
		close(c.done)
		c.signals.close()
		c.presence.close()
		c.errors.close()
	})
}

// LiveJoin joins the instance of the channel name that args name, showing
// presence to the others, or the channel's default when nil. fields are the
// join arguments in the order the server declares them. The join is made
// again after a reconnect, until Leave, or until ctx is done.
func LiveJoin[M any, P any](ctx context.Context, l *LiveConnection, name string, fields []string, args any, presence *P) (*LiveChannel[M, P], error) {
	key, err := LiveChannelKey(fields, args)
	if err != nil {
		return nil, &LiveError{Message: err.Error()}
	}
	payload, err := livePresent(args)
	if err != nil {
		return nil, err
	}
	state := json.RawMessage("null")
	if presence != nil {
		if state, err = json.Marshal(*presence); err != nil {
			return nil, &LiveError{Message: err.Error()}
		}
	}
	c := &LiveChannel[M, P]{
		l:        l,
		key:      key,
		signals:  newLiveQueue[LiveSignal[M]](false),
		presence: newLiveQueue[[]LiveMember[P]](true),
		errors:   newLiveQueue[*LiveError](false),
		done:     make(chan struct{}),
	}
	c.joined = &liveJoined{
		name:  name,
		args:  payload,
		state: state,
		onSignal: func(raw json.RawMessage, from string) {
			var message M
			if err := json.Unmarshal(raw, &message); err != nil {
				c.errors.push(&LiveError{Message: fmt.Sprintf("the signal was not the expected JSON: %v", err)})
				return
			}
			c.signals.push(LiveSignal[M]{Message: message, From: from})
		},
		onPresence: func(raw json.RawMessage) {
			var members []LiveMember[P]
			if err := json.Unmarshal(raw, &members); err != nil {
				c.errors.push(&LiveError{Message: fmt.Sprintf("the presence was not the expected JSON: %v", err)})
				return
			}
			c.mu.Lock()
			c.members = members
			c.mu.Unlock()
			c.presence.push(members)
		},
		onError: c.errors.push,
		onClose: c.shut,
	}
	defer l.flush()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		c.shut()
		return nil, liveClosed()
	}
	l.channels[key] = c.joined
	l.sendLocked(liveJoinFrame(key, c.joined))
	l.ensureLocked()
	go func() {
		select {
		case <-ctx.Done():
			c.Leave()
		case <-c.done:
		}
	}()
	return c, nil
}

// liveClient is the live connection a Client opens on first use.
type liveClient struct {
	mu   sync.Mutex
	conn *LiveConnection
}

// Live returns the client's live connection: one WebSocket for every
// subscription, live mutation and channel, opened on first use with
// LiveOptions. Watch Live().OnStatus for a connection banner, and call
// Live().Close when done.
func (c *Client) Live() *LiveConnection {
	c.live.mu.Lock()
	defer c.live.mu.Unlock()
	if c.live.conn == nil {
		c.live.conn = NewLiveConnection(LiveURL(c.ServerURL, LivePath), c.LiveOptions)
	}
	return c.live.conn
}
