"""Request and response types for the Chorus API."""

from __future__ import annotations

from dataclasses import dataclass, field


# --- Request types ---


@dataclass
class SendSmsRequest:
    to: str
    body: str
    from_: str | None = field(default=None, metadata={"alias": "from"})


@dataclass
class SendEmailRequest:
    to: str
    subject: str
    body: str
    from_: str | None = field(default=None, metadata={"alias": "from"})


@dataclass
class OtpSendRequest:
    to: str
    app_name: str | None = None


@dataclass
class OtpVerifyRequest:
    to: str
    code: str


@dataclass
class CreateWebhookRequest:
    url: str
    events: list[str]


# --- Response types ---


@dataclass
class SendResponse:
    message_id: str
    status: str


@dataclass
class BatchMessageResult:
    message_id: str
    to: str
    status: str


@dataclass
class BatchSendResponse:
    messages: list[BatchMessageResult]
    error: str | None = None


@dataclass
class OtpSendResponse:
    message_id: str
    expires_in: int


@dataclass
class OtpVerifyResponse:
    valid: bool


@dataclass
class Message:
    id: str
    account_id: str
    channel: str
    recipient: str
    body: str
    status: str
    environment: str
    created_at: str
    provider: str | None = None
    sender: str | None = None
    subject: str | None = None
    error_message: str | None = None
    delivered_at: str | None = None


@dataclass
class WebhookResponse:
    id: str
    url: str
    secret: str
    events: list[str]
    created_at: str


@dataclass
class WebhookListItem:
    id: str
    url: str
    events: list[str]
    created_at: str
