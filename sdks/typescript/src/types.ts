// --- Request types ---

export interface SendSmsRequest {
  to: string;
  body: string;
  from?: string;
}

export interface SendEmailRequest {
  to: string;
  subject: string;
  body: string;
  from?: string;
}

export interface SmsBatchRecipient {
  to: string;
  body: string;
}

export interface SendSmsBatchRequest {
  recipients: SmsBatchRecipient[];
  from?: string;
}

export interface EmailBatchRecipient {
  to: string;
  subject: string;
  body: string;
}

export interface SendEmailBatchRequest {
  recipients: EmailBatchRecipient[];
  from?: string;
}

export interface OtpSendRequest {
  to: string;
  app_name?: string;
}

export interface OtpVerifyRequest {
  to: string;
  code: string;
}

export interface CreateWebhookRequest {
  url: string;
  events: string[];
}

export interface ListMessagesParams {
  limit?: number;
  offset?: number;
}

// --- Response types ---

export interface SendResponse {
  message_id: string;
  status: string;
}

export interface BatchMessageResult {
  message_id: string;
  to: string;
  status: string;
}

export interface BatchSendResponse {
  messages: BatchMessageResult[];
  error?: string;
}

export interface OtpSendResponse {
  message_id: string;
  expires_in: number;
}

export interface OtpVerifyResponse {
  valid: boolean;
}

export interface Message {
  id: string;
  account_id: string;
  channel: string;
  provider?: string;
  sender?: string;
  recipient: string;
  subject?: string;
  body: string;
  status: string;
  error_message?: string;
  environment: string;
  created_at: string;
  delivered_at?: string;
}

export interface WebhookResponse {
  id: string;
  url: string;
  secret: string;
  events: string[];
  created_at: string;
}

export interface WebhookListItem {
  id: string;
  url: string;
  events: string[];
  created_at: string;
}

// --- Client config ---

export interface ChorusClientConfig {
  apiKey: string;
  baseUrl?: string;
}
