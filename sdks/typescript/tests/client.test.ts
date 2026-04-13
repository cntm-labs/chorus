import { describe, it, expect, beforeAll, afterAll, afterEach } from "vitest";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { ChorusClient, ChorusError } from "../src/index.js";

const BASE = "http://localhost:9999";

const handlers = [
  http.post(`${BASE}/v1/sms/send`, () =>
    HttpResponse.json({ message_id: "msg-1", status: "queued" }, { status: 202 })
  ),
  http.post(`${BASE}/v1/email/send`, () =>
    HttpResponse.json({ message_id: "msg-2", status: "queued" }, { status: 202 })
  ),
  http.post(`${BASE}/v1/sms/send-batch`, () =>
    HttpResponse.json({
      messages: [{ message_id: "msg-3", to: "+111", status: "queued" }],
    }, { status: 202 })
  ),
  http.post(`${BASE}/v1/email/send-batch`, () =>
    HttpResponse.json({
      messages: [{ message_id: "msg-4", to: "a@b.com", status: "queued" }],
    }, { status: 202 })
  ),
  http.post(`${BASE}/v1/otp/send`, () =>
    HttpResponse.json({ message_id: "msg-5", expires_in: 300 }, { status: 202 })
  ),
  http.post(`${BASE}/v1/otp/verify`, () =>
    HttpResponse.json({ valid: true })
  ),
  http.get(`${BASE}/v1/messages/msg-1`, () =>
    HttpResponse.json({
      id: "msg-1", account_id: "acc-1", channel: "sms",
      recipient: "+111", body: "hi", status: "delivered",
      environment: "live", created_at: "2026-01-01T00:00:00Z",
    })
  ),
  http.get(`${BASE}/v1/messages`, () =>
    HttpResponse.json([])
  ),
  http.post(`${BASE}/v1/webhooks`, () =>
    HttpResponse.json({
      id: "wh-1", url: "https://example.com/hook",
      secret: "abc123", events: ["message.delivered"],
      created_at: "2026-01-01T00:00:00Z",
    }, { status: 201 })
  ),
  http.get(`${BASE}/v1/webhooks`, () =>
    HttpResponse.json([])
  ),
  http.delete(`${BASE}/v1/webhooks/wh-1`, () =>
    new HttpResponse(null, { status: 204 })
  ),
];

const server = setupServer(...handlers);

beforeAll(() => server.listen());
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

const client = new ChorusClient({ apiKey: "ch_test_xxx", baseUrl: BASE });

describe("ChorusClient", () => {
  it("sends SMS", async () => {
    const res = await client.sms.send({ to: "+111", body: "hi" });
    expect(res.message_id).toBe("msg-1");
    expect(res.status).toBe("queued");
  });

  it("sends email", async () => {
    const res = await client.email.send({ to: "a@b.com", subject: "Hi", body: "Hello" });
    expect(res.message_id).toBe("msg-2");
  });

  it("sends SMS batch", async () => {
    const res = await client.sms.sendBatch({
      recipients: [{ to: "+111", body: "hi" }],
    });
    expect(res.messages).toHaveLength(1);
  });

  it("sends email batch", async () => {
    const res = await client.email.sendBatch({
      recipients: [{ to: "a@b.com", subject: "Hi", body: "Hello" }],
    });
    expect(res.messages).toHaveLength(1);
  });

  it("sends OTP", async () => {
    const res = await client.otp.send({ to: "+111" });
    expect(res.expires_in).toBe(300);
  });

  it("verifies OTP", async () => {
    const res = await client.otp.verify({ to: "+111", code: "123456" });
    expect(res.valid).toBe(true);
  });

  it("gets message", async () => {
    const msg = await client.messages.get("msg-1");
    expect(msg.id).toBe("msg-1");
    expect(msg.status).toBe("delivered");
  });

  it("lists messages", async () => {
    const msgs = await client.messages.list();
    expect(msgs).toEqual([]);
  });

  it("creates webhook", async () => {
    const wh = await client.webhooks.create({
      url: "https://example.com/hook",
      events: ["message.delivered"],
    });
    expect(wh.id).toBe("wh-1");
    expect(wh.secret).toBe("abc123");
  });

  it("lists webhooks", async () => {
    const whs = await client.webhooks.list();
    expect(whs).toEqual([]);
  });

  it("deletes webhook", async () => {
    await client.webhooks.delete("wh-1");
  });

  it("throws ChorusError on API error", async () => {
    server.use(
      http.post(`${BASE}/v1/sms/send`, () =>
        HttpResponse.text("Unauthorized", { status: 401 })
      )
    );

    await expect(client.sms.send({ to: "+111", body: "hi" }))
      .rejects.toThrow(ChorusError);
  });
});
