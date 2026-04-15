"""Tests for ChorusClient using pytest-httpserver."""

import pytest
from chorus import ChorusClient, ChorusError
from chorus.types import (
    CreateWebhookRequest,
    OtpSendRequest,
    OtpVerifyRequest,
    SendEmailRequest,
    SendSmsRequest,
)
from pytest_httpserver import HTTPServer


@pytest.fixture()
def server(httpserver: HTTPServer) -> HTTPServer:
    httpserver.expect_request("/v1/sms/send", method="POST").respond_with_json(
        {"message_id": "msg-1", "status": "queued"}, status=202
    )
    httpserver.expect_request("/v1/email/send", method="POST").respond_with_json(
        {"message_id": "msg-2", "status": "queued"}, status=202
    )
    httpserver.expect_request("/v1/sms/send-batch", method="POST").respond_with_json(
        {"messages": [{"message_id": "msg-3", "to": "+111", "status": "queued"}]}, status=202
    )
    httpserver.expect_request("/v1/otp/send", method="POST").respond_with_json(
        {"message_id": "msg-4", "expires_in": 300}, status=202
    )
    httpserver.expect_request("/v1/otp/verify", method="POST").respond_with_json(
        {"valid": True}
    )
    httpserver.expect_request("/v1/messages/msg-1", method="GET").respond_with_json(
        {
            "id": "msg-1", "account_id": "acc-1", "channel": "sms",
            "recipient": "+111", "body": "hi", "status": "delivered",
            "environment": "live", "created_at": "2026-01-01T00:00:00Z",
        }
    )
    httpserver.expect_request("/v1/messages", method="GET").respond_with_json([])
    httpserver.expect_request("/v1/webhooks", method="POST").respond_with_json(
        {
            "id": "wh-1", "url": "https://example.com/hook",
            "secret": "abc123", "events": ["message.delivered"],
            "created_at": "2026-01-01T00:00:00Z",
        },
        status=201,
    )
    httpserver.expect_request("/v1/webhooks", method="GET").respond_with_json([])
    httpserver.expect_request("/v1/webhooks/wh-1", method="DELETE").respond_with_data(
        "", status=204
    )
    return httpserver


@pytest.fixture()
def client(server: HTTPServer) -> ChorusClient:
    return ChorusClient("ch_test_xxx", base_url=server.url_for(""))


def test_send_sms(client: ChorusClient) -> None:
    res = client.sms.send(SendSmsRequest(to="+111", body="hi"))
    assert res.message_id == "msg-1"
    assert res.status == "queued"


def test_send_email(client: ChorusClient) -> None:
    res = client.email.send(SendEmailRequest(to="a@b.com", subject="Hi", body="Hello"))
    assert res.message_id == "msg-2"


def test_send_sms_batch(client: ChorusClient) -> None:
    res = client.sms.send_batch([{"to": "+111", "body": "hi"}])
    assert len(res.messages) == 1


def test_send_otp(client: ChorusClient) -> None:
    res = client.otp.send(OtpSendRequest(to="+111"))
    assert res.expires_in == 300


def test_verify_otp(client: ChorusClient) -> None:
    res = client.otp.verify(OtpVerifyRequest(to="+111", code="123456"))
    assert res.valid is True


def test_get_message(client: ChorusClient) -> None:
    msg = client.messages.get("msg-1")
    assert msg.id == "msg-1"
    assert msg.status == "delivered"


def test_list_messages(client: ChorusClient) -> None:
    msgs = client.messages.list()
    assert msgs == []


def test_create_webhook(client: ChorusClient) -> None:
    wh = client.webhooks.create(CreateWebhookRequest(url="https://example.com/hook", events=["message.delivered"]))
    assert wh.id == "wh-1"
    assert wh.secret == "abc123"


def test_delete_webhook(client: ChorusClient) -> None:
    client.webhooks.delete("wh-1")


def test_api_error(httpserver: HTTPServer) -> None:
    httpserver.expect_request("/v1/sms/send", method="POST").respond_with_data(
        "Unauthorized", status=401
    )
    c = ChorusClient("bad_key", base_url=httpserver.url_for(""))
    with pytest.raises(ChorusError) as exc_info:
        c.sms.send(SendSmsRequest(to="+111", body="hi"))
    assert exc_info.value.status == 401
