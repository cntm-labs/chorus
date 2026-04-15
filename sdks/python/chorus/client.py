"""Chorus CPaaS client."""

from __future__ import annotations

from typing import Any

import httpx

from chorus.errors import ChorusError
from chorus.types import (
    BatchSendResponse,
    BatchMessageResult,
    CreateWebhookRequest,
    Message,
    OtpSendRequest,
    OtpSendResponse,
    OtpVerifyRequest,
    OtpVerifyResponse,
    SendEmailRequest,
    SendResponse,
    SendSmsRequest,
    WebhookListItem,
    WebhookResponse,
)

_DEFAULT_BASE_URL = "http://localhost:3000"


class ChorusClient:
    """Chorus CPaaS client."""

    def __init__(self, api_key: str, *, base_url: str = _DEFAULT_BASE_URL) -> None:
        self._api_key = api_key
        self._base_url = base_url.rstrip("/")
        self._http = httpx.Client(
            base_url=self._base_url,
            headers={
                "Authorization": f"Bearer {self._api_key}",
                "Content-Type": "application/json",
            },
        )
        self.sms = _SmsClient(self)
        self.email = _EmailClient(self)
        self.otp = _OtpClient(self)
        self.messages = _MessageClient(self)
        self.webhooks = _WebhookClient(self)

    def close(self) -> None:
        """Close the underlying HTTP client."""
        self._http.close()

    def __enter__(self) -> ChorusClient:
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()

    def _request(self, method: str, path: str, *, json: Any = None) -> Any:
        resp = self._http.request(method, path, json=json)
        if resp.status_code >= 400:
            raise ChorusError(resp.status_code, resp.text)
        if resp.status_code == 204 or not resp.content:
            return None
        return resp.json()


def _to_json(req: Any) -> dict[str, Any]:
    """Convert a dataclass to a JSON-safe dict, handling from_ → from."""
    d: dict[str, Any] = {}
    for k, v in req.__dict__.items():
        if v is None:
            continue
        key = "from" if k == "from_" else k
        d[key] = v
    return d


class _SmsClient:
    def __init__(self, client: ChorusClient) -> None:
        self._c = client

    def send(self, req: SendSmsRequest) -> SendResponse:
        data = self._c._request("POST", "/v1/sms/send", json=_to_json(req))
        return SendResponse(**data)

    def send_batch(self, recipients: list[dict[str, str]], from_: str | None = None) -> BatchSendResponse:
        body: dict[str, Any] = {"recipients": recipients}
        if from_:
            body["from"] = from_
        data = self._c._request("POST", "/v1/sms/send-batch", json=body)
        return BatchSendResponse(
            messages=[BatchMessageResult(**m) for m in data["messages"]],
            error=data.get("error"),
        )


class _EmailClient:
    def __init__(self, client: ChorusClient) -> None:
        self._c = client

    def send(self, req: SendEmailRequest) -> SendResponse:
        data = self._c._request("POST", "/v1/email/send", json=_to_json(req))
        return SendResponse(**data)

    def send_batch(self, recipients: list[dict[str, str]], from_: str | None = None) -> BatchSendResponse:
        body: dict[str, Any] = {"recipients": recipients}
        if from_:
            body["from"] = from_
        data = self._c._request("POST", "/v1/email/send-batch", json=body)
        return BatchSendResponse(
            messages=[BatchMessageResult(**m) for m in data["messages"]],
            error=data.get("error"),
        )


class _OtpClient:
    def __init__(self, client: ChorusClient) -> None:
        self._c = client

    def send(self, req: OtpSendRequest) -> OtpSendResponse:
        data = self._c._request("POST", "/v1/otp/send", json=_to_json(req))
        return OtpSendResponse(**data)

    def verify(self, req: OtpVerifyRequest) -> OtpVerifyResponse:
        data = self._c._request("POST", "/v1/otp/verify", json=_to_json(req))
        return OtpVerifyResponse(**data)


class _MessageClient:
    def __init__(self, client: ChorusClient) -> None:
        self._c = client

    def get(self, id: str) -> Message:
        data = self._c._request("GET", f"/v1/messages/{id}")
        return Message(**data)

    def list(self, *, limit: int = 20, offset: int = 0) -> list[Message]:
        data = self._c._request("GET", f"/v1/messages?limit={limit}&offset={offset}")
        return [Message(**m) for m in data]


class _WebhookClient:
    def __init__(self, client: ChorusClient) -> None:
        self._c = client

    def create(self, req: CreateWebhookRequest) -> WebhookResponse:
        data = self._c._request("POST", "/v1/webhooks", json=_to_json(req))
        return WebhookResponse(**data)

    def list(self) -> list[WebhookListItem]:
        data = self._c._request("GET", "/v1/webhooks")
        return [WebhookListItem(**w) for w in data]

    def delete(self, id: str) -> None:
        self._c._request("DELETE", f"/v1/webhooks/{id}")
