// Package chorus provides a Go client for the Chorus CPaaS API.
package chorus

// --- Request types ---

// SendSmsRequest is the payload for sending a single SMS.
type SendSmsRequest struct {
	To   string `json:"to"`
	Body string `json:"body"`
	From string `json:"from,omitempty"`
}

// SendEmailRequest is the payload for sending a single email.
type SendEmailRequest struct {
	To      string `json:"to"`
	Subject string `json:"subject"`
	Body    string `json:"body"`
	From    string `json:"from,omitempty"`
}

// SmsBatchRecipient is a single recipient in a batch SMS request.
type SmsBatchRecipient struct {
	To   string `json:"to"`
	Body string `json:"body"`
}

// SendSmsBatchRequest is the payload for batch SMS.
type SendSmsBatchRequest struct {
	Recipients []SmsBatchRecipient `json:"recipients"`
	From       string              `json:"from,omitempty"`
}

// EmailBatchRecipient is a single recipient in a batch email request.
type EmailBatchRecipient struct {
	To      string `json:"to"`
	Subject string `json:"subject"`
	Body    string `json:"body"`
}

// SendEmailBatchRequest is the payload for batch email.
type SendEmailBatchRequest struct {
	Recipients []EmailBatchRecipient `json:"recipients"`
	From       string                `json:"from,omitempty"`
}

// OtpSendRequest is the payload for sending an OTP.
type OtpSendRequest struct {
	To      string `json:"to"`
	AppName string `json:"app_name,omitempty"`
}

// OtpVerifyRequest is the payload for verifying an OTP.
type OtpVerifyRequest struct {
	To   string `json:"to"`
	Code string `json:"code"`
}

// CreateWebhookRequest is the payload for registering a webhook.
type CreateWebhookRequest struct {
	URL    string   `json:"url"`
	Events []string `json:"events"`
}

// --- Response types ---

// SendResponse is returned after queuing a message.
type SendResponse struct {
	MessageID string `json:"message_id"`
	Status    string `json:"status"`
}

// BatchMessageResult is a single result in a batch send response.
type BatchMessageResult struct {
	MessageID string `json:"message_id"`
	To        string `json:"to"`
	Status    string `json:"status"`
}

// BatchSendResponse is returned after a batch send.
type BatchSendResponse struct {
	Messages []BatchMessageResult `json:"messages"`
	Error    string               `json:"error,omitempty"`
}

// OtpSendResponse is returned after sending an OTP.
type OtpSendResponse struct {
	MessageID string `json:"message_id"`
	ExpiresIn int    `json:"expires_in"`
}

// OtpVerifyResponse is returned after verifying an OTP.
type OtpVerifyResponse struct {
	Valid bool `json:"valid"`
}

// Message represents a stored message with delivery status.
type Message struct {
	ID           string  `json:"id"`
	AccountID    string  `json:"account_id"`
	Channel      string  `json:"channel"`
	Provider     *string `json:"provider,omitempty"`
	Sender       *string `json:"sender,omitempty"`
	Recipient    string  `json:"recipient"`
	Subject      *string `json:"subject,omitempty"`
	Body         string  `json:"body"`
	Status       string  `json:"status"`
	ErrorMessage *string `json:"error_message,omitempty"`
	Environment  string  `json:"environment"`
	CreatedAt    string  `json:"created_at"`
	DeliveredAt  *string `json:"delivered_at,omitempty"`
}

// WebhookResponse is returned after creating a webhook.
type WebhookResponse struct {
	ID        string   `json:"id"`
	URL       string   `json:"url"`
	Secret    string   `json:"secret"`
	Events    []string `json:"events"`
	CreatedAt string   `json:"created_at"`
}

// WebhookListItem is a webhook in a list (secret redacted).
type WebhookListItem struct {
	ID        string   `json:"id"`
	URL       string   `json:"url"`
	Events    []string `json:"events"`
	CreatedAt string   `json:"created_at"`
}
