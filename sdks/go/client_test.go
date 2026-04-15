package chorus

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func setupServer() *httptest.Server {
	mux := http.NewServeMux()

	mux.HandleFunc("POST /v1/sms/send", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(202)
		json.NewEncoder(w).Encode(SendResponse{MessageID: "msg-1", Status: "queued"})
	})
	mux.HandleFunc("POST /v1/email/send", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(202)
		json.NewEncoder(w).Encode(SendResponse{MessageID: "msg-2", Status: "queued"})
	})
	mux.HandleFunc("POST /v1/sms/send-batch", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(202)
		json.NewEncoder(w).Encode(BatchSendResponse{
			Messages: []BatchMessageResult{{MessageID: "msg-3", To: "+111", Status: "queued"}},
		})
	})
	mux.HandleFunc("POST /v1/otp/send", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(202)
		json.NewEncoder(w).Encode(OtpSendResponse{MessageID: "msg-4", ExpiresIn: 300})
	})
	mux.HandleFunc("POST /v1/otp/verify", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(OtpVerifyResponse{Valid: true})
	})
	mux.HandleFunc("GET /v1/messages/msg-1", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(Message{
			ID: "msg-1", AccountID: "acc-1", Channel: "sms",
			Recipient: "+111", Body: "hi", Status: "delivered",
			Environment: "live", CreatedAt: "2026-01-01T00:00:00Z",
		})
	})
	mux.HandleFunc("GET /v1/messages", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode([]Message{})
	})
	mux.HandleFunc("POST /v1/webhooks", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(201)
		json.NewEncoder(w).Encode(WebhookResponse{
			ID: "wh-1", URL: "https://example.com/hook",
			Secret: "abc123", Events: []string{"message.delivered"},
			CreatedAt: "2026-01-01T00:00:00Z",
		})
	})
	mux.HandleFunc("GET /v1/webhooks", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode([]WebhookListItem{})
	})
	mux.HandleFunc("DELETE /v1/webhooks/wh-1", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(204)
	})

	return httptest.NewServer(mux)
}

func TestSendSms(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	res, err := c.Sms.Send(SendSmsRequest{To: "+111", Body: "hi"})
	if err != nil {
		t.Fatal(err)
	}
	if res.MessageID != "msg-1" {
		t.Errorf("expected msg-1, got %s", res.MessageID)
	}
}

func TestSendEmail(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	res, err := c.Email.Send(SendEmailRequest{To: "a@b.com", Subject: "Hi", Body: "Hello"})
	if err != nil {
		t.Fatal(err)
	}
	if res.MessageID != "msg-2" {
		t.Errorf("expected msg-2, got %s", res.MessageID)
	}
}

func TestSendSmsBatch(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	res, err := c.Sms.SendBatch(SendSmsBatchRequest{
		Recipients: []SmsBatchRecipient{{To: "+111", Body: "hi"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Messages) != 1 {
		t.Errorf("expected 1 message, got %d", len(res.Messages))
	}
}

func TestSendOtp(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	res, err := c.Otp.Send(OtpSendRequest{To: "+111"})
	if err != nil {
		t.Fatal(err)
	}
	if res.ExpiresIn != 300 {
		t.Errorf("expected 300, got %d", res.ExpiresIn)
	}
}

func TestVerifyOtp(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	res, err := c.Otp.Verify(OtpVerifyRequest{To: "+111", Code: "123456"})
	if err != nil {
		t.Fatal(err)
	}
	if !res.Valid {
		t.Error("expected valid=true")
	}
}

func TestGetMessage(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	msg, err := c.Messages.Get("msg-1")
	if err != nil {
		t.Fatal(err)
	}
	if msg.ID != "msg-1" {
		t.Errorf("expected msg-1, got %s", msg.ID)
	}
}

func TestListMessages(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	msgs, err := c.Messages.List(10, 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(msgs) != 0 {
		t.Errorf("expected empty list, got %d", len(msgs))
	}
}

func TestCreateWebhook(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	wh, err := c.Webhooks.Create(CreateWebhookRequest{
		URL: "https://example.com/hook", Events: []string{"message.delivered"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if wh.ID != "wh-1" {
		t.Errorf("expected wh-1, got %s", wh.ID)
	}
	if wh.Secret != "abc123" {
		t.Errorf("expected abc123, got %s", wh.Secret)
	}
}

func TestDeleteWebhook(t *testing.T) {
	srv := setupServer()
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	err := c.Webhooks.Delete("wh-1")
	if err != nil {
		t.Fatal(err)
	}
}

func TestApiError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(401)
		w.Write([]byte("Unauthorized"))
	}))
	defer srv.Close()
	c := NewClient("ch_test_xxx", WithBaseURL(srv.URL))

	_, err := c.Sms.Send(SendSmsRequest{To: "+111", Body: "hi"})
	if err == nil {
		t.Fatal("expected error")
	}
	chorusErr, ok := err.(*ChorusError)
	if !ok {
		t.Fatal("expected ChorusError")
	}
	if chorusErr.Status != 401 {
		t.Errorf("expected 401, got %d", chorusErr.Status)
	}
}
