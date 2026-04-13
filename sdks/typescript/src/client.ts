import { ChorusError } from "./errors.js";
import type {
  BatchSendResponse,
  ChorusClientConfig,
  CreateWebhookRequest,
  ListMessagesParams,
  Message,
  OtpSendRequest,
  OtpSendResponse,
  OtpVerifyRequest,
  OtpVerifyResponse,
  SendEmailBatchRequest,
  SendEmailRequest,
  SendResponse,
  SendSmsBatchRequest,
  SendSmsRequest,
  WebhookListItem,
  WebhookResponse,
} from "./types.js";

const DEFAULT_BASE_URL = "http://localhost:3000";

/** Chorus CPaaS client. */
export class ChorusClient {
  private readonly apiKey: string;
  private readonly baseUrl: string;

  /** SMS operations. */
  readonly sms: SmsClient;
  /** Email operations. */
  readonly email: EmailClient;
  /** OTP operations. */
  readonly otp: OtpClient;
  /** Message queries. */
  readonly messages: MessageClient;
  /** Webhook management. */
  readonly webhooks: WebhookClient;

  constructor(config: ChorusClientConfig) {
    this.apiKey = config.apiKey;
    this.baseUrl = (config.baseUrl ?? DEFAULT_BASE_URL).replace(/\/+$/, "");
    this.sms = new SmsClient(this);
    this.email = new EmailClient(this);
    this.otp = new OtpClient(this);
    this.messages = new MessageClient(this);
    this.webhooks = new WebhookClient(this);
  }

  /** Send a request to the Chorus API. */
  async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const url = `${this.baseUrl}${path}`;
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.apiKey}`,
      "Content-Type": "application/json",
    };

    const resp = await fetch(url, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });

    if (!resp.ok) {
      const text = await resp.text();
      throw new ChorusError(resp.status, text);
    }

    if (resp.status === 204) {
      return undefined as T;
    }

    return (await resp.json()) as T;
  }
}

class SmsClient {
  constructor(private readonly client: ChorusClient) {}

  /** Send a single SMS. */
  async send(req: SendSmsRequest): Promise<SendResponse> {
    return this.client.request("POST", "/v1/sms/send", req);
  }

  /** Send SMS to multiple recipients. */
  async sendBatch(req: SendSmsBatchRequest): Promise<BatchSendResponse> {
    return this.client.request("POST", "/v1/sms/send-batch", req);
  }
}

class EmailClient {
  constructor(private readonly client: ChorusClient) {}

  /** Send a single email. */
  async send(req: SendEmailRequest): Promise<SendResponse> {
    return this.client.request("POST", "/v1/email/send", req);
  }

  /** Send email to multiple recipients. */
  async sendBatch(req: SendEmailBatchRequest): Promise<BatchSendResponse> {
    return this.client.request("POST", "/v1/email/send-batch", req);
  }
}

class OtpClient {
  constructor(private readonly client: ChorusClient) {}

  /** Send an OTP code. */
  async send(req: OtpSendRequest): Promise<OtpSendResponse> {
    return this.client.request("POST", "/v1/otp/send", req);
  }

  /** Verify an OTP code. */
  async verify(req: OtpVerifyRequest): Promise<OtpVerifyResponse> {
    return this.client.request("POST", "/v1/otp/verify", req);
  }
}

class MessageClient {
  constructor(private readonly client: ChorusClient) {}

  /** Get a message by ID. */
  async get(id: string): Promise<Message> {
    return this.client.request("GET", `/v1/messages/${id}`);
  }

  /** List messages. */
  async list(params?: ListMessagesParams): Promise<Message[]> {
    const query = new URLSearchParams();
    if (params?.limit !== undefined) query.set("limit", String(params.limit));
    if (params?.offset !== undefined) query.set("offset", String(params.offset));
    const qs = query.toString();
    return this.client.request("GET", `/v1/messages${qs ? `?${qs}` : ""}`);
  }
}

class WebhookClient {
  constructor(private readonly client: ChorusClient) {}

  /** Register a new webhook. */
  async create(req: CreateWebhookRequest): Promise<WebhookResponse> {
    return this.client.request("POST", "/v1/webhooks", req);
  }

  /** List all active webhooks. */
  async list(): Promise<WebhookListItem[]> {
    return this.client.request("GET", "/v1/webhooks");
  }

  /** Delete a webhook. */
  async delete(id: string): Promise<void> {
    return this.client.request("DELETE", `/v1/webhooks/${id}`);
  }
}
