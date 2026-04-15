package chorus

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
)

const defaultBaseURL = "http://localhost:3000"

// Client is the Chorus CPaaS API client.
type Client struct {
	apiKey  string
	baseURL string
	http    *http.Client
	Sms     *SmsClient
	Email   *EmailClient
	Otp     *OtpClient
	Messages *MessageClient
	Webhooks *WebhookClient
}

// NewClient creates a new Chorus client with the given API key.
func NewClient(apiKey string, opts ...Option) *Client {
	c := &Client{
		apiKey:  apiKey,
		baseURL: defaultBaseURL,
		http:    http.DefaultClient,
	}
	for _, opt := range opts {
		opt(c)
	}
	c.Sms = &SmsClient{client: c}
	c.Email = &EmailClient{client: c}
	c.Otp = &OtpClient{client: c}
	c.Messages = &MessageClient{client: c}
	c.Webhooks = &WebhookClient{client: c}
	return c
}

// Option configures a Client.
type Option func(*Client)

// WithBaseURL sets a custom base URL.
func WithBaseURL(url string) Option {
	return func(c *Client) {
		c.baseURL = strings.TrimRight(url, "/")
	}
}

// WithHTTPClient sets a custom HTTP client.
func WithHTTPClient(hc *http.Client) Option {
	return func(c *Client) {
		c.http = hc
	}
}

func (c *Client) request(method, path string, body, result interface{}) error {
	url := c.baseURL + path

	var reqBody io.Reader
	if body != nil {
		data, err := json.Marshal(body)
		if err != nil {
			return err
		}
		reqBody = bytes.NewReader(data)
	}

	req, err := http.NewRequest(method, url, reqBody)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.apiKey)
	req.Header.Set("Content-Type", "application/json")

	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	respBody, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}

	if resp.StatusCode >= 400 {
		return &ChorusError{Status: resp.StatusCode, Body: string(respBody)}
	}

	if result != nil && len(respBody) > 0 {
		return json.Unmarshal(respBody, result)
	}
	return nil
}

// --- Sub-clients ---

// SmsClient handles SMS operations.
type SmsClient struct{ client *Client }

// Send sends a single SMS.
func (s *SmsClient) Send(req SendSmsRequest) (*SendResponse, error) {
	var resp SendResponse
	err := s.client.request("POST", "/v1/sms/send", req, &resp)
	return &resp, err
}

// SendBatch sends SMS to multiple recipients.
func (s *SmsClient) SendBatch(req SendSmsBatchRequest) (*BatchSendResponse, error) {
	var resp BatchSendResponse
	err := s.client.request("POST", "/v1/sms/send-batch", req, &resp)
	return &resp, err
}

// EmailClient handles email operations.
type EmailClient struct{ client *Client }

// Send sends a single email.
func (e *EmailClient) Send(req SendEmailRequest) (*SendResponse, error) {
	var resp SendResponse
	err := e.client.request("POST", "/v1/email/send", req, &resp)
	return &resp, err
}

// SendBatch sends email to multiple recipients.
func (e *EmailClient) SendBatch(req SendEmailBatchRequest) (*BatchSendResponse, error) {
	var resp BatchSendResponse
	err := e.client.request("POST", "/v1/email/send-batch", req, &resp)
	return &resp, err
}

// OtpClient handles OTP operations.
type OtpClient struct{ client *Client }

// Send sends an OTP code.
func (o *OtpClient) Send(req OtpSendRequest) (*OtpSendResponse, error) {
	var resp OtpSendResponse
	err := o.client.request("POST", "/v1/otp/send", req, &resp)
	return &resp, err
}

// Verify verifies an OTP code.
func (o *OtpClient) Verify(req OtpVerifyRequest) (*OtpVerifyResponse, error) {
	var resp OtpVerifyResponse
	err := o.client.request("POST", "/v1/otp/verify", req, &resp)
	return &resp, err
}

// MessageClient handles message queries.
type MessageClient struct{ client *Client }

// Get retrieves a message by ID.
func (m *MessageClient) Get(id string) (*Message, error) {
	var resp Message
	err := m.client.request("GET", "/v1/messages/"+id, nil, &resp)
	return &resp, err
}

// List retrieves messages with optional pagination.
func (m *MessageClient) List(limit, offset int) ([]Message, error) {
	path := fmt.Sprintf("/v1/messages?limit=%d&offset=%d", limit, offset)
	var resp []Message
	err := m.client.request("GET", path, nil, &resp)
	return resp, err
}

// WebhookClient handles webhook management.
type WebhookClient struct{ client *Client }

// Create registers a new webhook.
func (w *WebhookClient) Create(req CreateWebhookRequest) (*WebhookResponse, error) {
	var resp WebhookResponse
	err := w.client.request("POST", "/v1/webhooks", req, &resp)
	return &resp, err
}

// List returns all active webhooks.
func (w *WebhookClient) List() ([]WebhookListItem, error) {
	var resp []WebhookListItem
	err := w.client.request("GET", "/v1/webhooks", nil, &resp)
	return resp, err
}

// Delete removes a webhook.
func (w *WebhookClient) Delete(id string) error {
	return w.client.request("DELETE", "/v1/webhooks/"+id, nil, nil)
}
