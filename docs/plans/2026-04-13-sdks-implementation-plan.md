# SDKs + v0.2.0 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create Rust, TypeScript, and Java SDKs for Chorus, update README, and bump to v0.2.0.

**Architecture:** Rust SDK re-exports chorus-core. TypeScript and Java SDKs are REST clients wrapping the Chorus HTTP API. Each SDK is a separate PR. Version bump is the final PR after all SDKs merge.

**Tech Stack:** Rust (re-export crate), TypeScript (native fetch, vitest), Java (java.net.http, Jackson, JUnit 5, WireMock)

---

## PR 1: Rust SDK

### Task 1: Create Rust SDK crate with re-exports

**Files:**
- Create: `sdks/rust/Cargo.toml`
- Create: `sdks/rust/src/lib.rs`
- Modify: `Cargo.toml` (add to workspace members)

**Step 1: Create directory structure**

```bash
mkdir -p sdks/rust/src
```

**Step 2: Create Cargo.toml**

In `sdks/rust/Cargo.toml`:

```toml
[package]
name = "chorus-sdk"
version = "0.2.0"
edition = "2021"
description = "Chorus CPaaS SDK — SMS, Email, OTP with smart routing"
license = "MIT"
repository = "https://github.com/cntm-labs/chorus"
readme = "README.md"
keywords = ["sms", "email", "otp", "cpas", "messaging"]
categories = ["api-bindings", "web-programming"]

[dependencies]
chorus-core = { path = "../../crates/chorus-core", version = "0.1.1" }

[lints]
workspace = true
```

**Step 3: Create lib.rs with re-exports and prelude**

In `sdks/rust/src/lib.rs`:

```rust
//! # chorus-sdk
//!
//! Official Rust SDK for Chorus — open-source CPaaS with SMS, Email, and OTP.
//!
//! This crate re-exports [`chorus-core`] for convenience. Use the [`prelude`]
//! module to import commonly used types.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use chorus_sdk::prelude::*;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ChorusError> {
//! let chorus = Chorus::builder()
//!     .default_from_sms("+1234567890".into())
//!     .build();
//!
//! let msg = SmsMessage {
//!     to: "+0987654321".into(),
//!     body: "Hello from Chorus!".into(),
//!     from: None,
//! };
//! let result = chorus.send_sms(&msg).await?;
//! # Ok(())
//! # }
//! ```

// Re-export all public modules from chorus-core.
pub use chorus::client;
pub use chorus::email;
pub use chorus::error;
pub use chorus::router;
pub use chorus::sms;
pub use chorus::template;
pub use chorus::templates;
pub use chorus::types;

/// Prelude module — import commonly used types with `use chorus_sdk::prelude::*`.
pub mod prelude {
    pub use chorus::client::Chorus;
    pub use chorus::email::EmailSender;
    pub use chorus::error::ChorusError;
    pub use chorus::router::WaterfallRouter;
    pub use chorus::sms::SmsSender;
    pub use chorus::types::{
        Channel, DeliveryStatus, EmailMessage, SendResult, SmsMessage, TemplateEmailMessage,
    };
}
```

**Step 4: Add to workspace**

In root `Cargo.toml`, add `"sdks/rust"` to `workspace.members`:

```toml
members = [
    "crates/chorus-core",
    "crates/chorus-providers",
    "crates/chorus-server",
    "sdks/rust",
]
```

**Step 5: Verify**

```bash
cargo check --workspace
cargo test -p chorus-sdk
```

**Step 6: Commit**

```bash
git add sdks/rust/ Cargo.toml Cargo.lock
git commit -m "feat(sdk): add Rust SDK with chorus-core re-exports and prelude"
```

---

### Task 2: Add Rust SDK tests

**Files:**
- Create: `sdks/rust/tests/prelude_test.rs`

**Step 1: Write test**

In `sdks/rust/tests/prelude_test.rs`:

```rust
use chorus_sdk::prelude::*;

#[test]
fn prelude_imports_all_key_types() {
    // Verify all prelude types are accessible
    let _msg = SmsMessage {
        to: "+1234567890".into(),
        body: "test".into(),
        from: None,
    };

    let _email = EmailMessage {
        to: "test@example.com".into(),
        subject: "Test".into(),
        html_body: "<p>Hi</p>".into(),
        text_body: "Hi".into(),
        from: None,
    };

    // Builder is accessible
    let _chorus = Chorus::builder().build();

    // Router is accessible
    let _router = WaterfallRouter::new();
}

#[test]
fn module_re_exports_work() {
    // Direct module access works
    let _: chorus_sdk::types::Channel = chorus_sdk::types::Channel::Sms;
}
```

**Step 2: Run test**

```bash
cargo test -p chorus-sdk
```
Expected: PASS

**Step 3: Commit**

```bash
git add sdks/rust/tests/
git commit -m "test(sdk): add Rust SDK prelude and re-export tests"
```

---

### Task 3: Lint, format, create PR for Rust SDK

**Step 1: Run CI checks**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

**Step 2: Create PR**

```bash
git checkout -b feat/rust-sdk
git push -u origin feat/rust-sdk
gh pr create --title "feat(sdk): Rust SDK with chorus-core re-exports" \
  --body "Adds sdks/rust/ crate (chorus-sdk) that re-exports chorus-core with a prelude module." \
  --label "enhancement" --assignee "MrBT-nano"
```

---

## PR 2: TypeScript SDK

### Task 4: Scaffold TypeScript SDK project

**Files:**
- Create: `sdks/typescript/package.json`
- Create: `sdks/typescript/tsconfig.json`
- Create: `sdks/typescript/.gitignore`

**Step 1: Create directory**

```bash
mkdir -p sdks/typescript/src sdks/typescript/tests
```

**Step 2: Create package.json**

In `sdks/typescript/package.json`:

```json
{
  "name": "@chorus/sdk",
  "version": "0.2.0",
  "description": "Official TypeScript SDK for Chorus CPaaS — SMS, Email, OTP",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "files": ["dist"],
  "scripts": {
    "build": "tsc",
    "test": "vitest run",
    "test:watch": "vitest",
    "lint": "tsc --noEmit"
  },
  "keywords": ["sms", "email", "otp", "cpas", "messaging", "chorus"],
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "https://github.com/cntm-labs/chorus",
    "directory": "sdks/typescript"
  },
  "engines": {
    "node": ">=18"
  },
  "devDependencies": {
    "typescript": "^5.4",
    "vitest": "^3.0",
    "msw": "^2.7"
  }
}
```

**Step 3: Create tsconfig.json**

In `sdks/typescript/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "Node16",
    "moduleResolution": "Node16",
    "lib": ["ES2022"],
    "outDir": "dist",
    "rootDir": "src",
    "declaration": true,
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true
  },
  "include": ["src"],
  "exclude": ["dist", "tests", "node_modules"]
}
```

**Step 4: Create .gitignore**

In `sdks/typescript/.gitignore`:

```
node_modules/
dist/
```

**Step 5: Commit**

```bash
git add sdks/typescript/
git commit -m "chore(sdk): scaffold TypeScript SDK project"
```

---

### Task 5: Implement TypeScript SDK types and errors

**Files:**
- Create: `sdks/typescript/src/types.ts`
- Create: `sdks/typescript/src/errors.ts`

**Step 1: Create types.ts**

In `sdks/typescript/src/types.ts`:

```typescript
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
```

**Step 2: Create errors.ts**

In `sdks/typescript/src/errors.ts`:

```typescript
/** Error returned by the Chorus API. */
export class ChorusError extends Error {
  /** HTTP status code. */
  readonly status: number;
  /** Raw response body. */
  readonly body: string;

  constructor(status: number, body: string) {
    super(`Chorus API error (${status}): ${body}`);
    this.name = "ChorusError";
    this.status = status;
    this.body = body;
  }
}
```

**Step 3: Commit**

```bash
git add sdks/typescript/src/types.ts sdks/typescript/src/errors.ts
git commit -m "feat(sdk): add TypeScript SDK types and error class"
```

---

### Task 6: Implement TypeScript SDK client

**Files:**
- Create: `sdks/typescript/src/client.ts`
- Create: `sdks/typescript/src/index.ts`

**Step 1: Create client.ts**

In `sdks/typescript/src/client.ts`:

```typescript
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
```

**Step 2: Create index.ts**

In `sdks/typescript/src/index.ts`:

```typescript
export { ChorusClient } from "./client.js";
export { ChorusError } from "./errors.js";
export type * from "./types.js";
```

**Step 3: Commit**

```bash
git add sdks/typescript/src/client.ts sdks/typescript/src/index.ts
git commit -m "feat(sdk): implement TypeScript SDK client with all endpoints"
```

---

### Task 7: Add TypeScript SDK tests

**Files:**
- Create: `sdks/typescript/tests/client.test.ts`
- Create: `sdks/typescript/vitest.config.ts`

**Step 1: Create vitest config**

In `sdks/typescript/vitest.config.ts`:

```typescript
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globals: true,
  },
});
```

**Step 2: Create tests**

In `sdks/typescript/tests/client.test.ts`:

```typescript
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
```

**Step 3: Install deps and run tests**

```bash
cd sdks/typescript && npm install && npm test
```
Expected: All 12 tests PASS

**Step 4: Commit**

```bash
git add sdks/typescript/
git commit -m "test(sdk): add TypeScript SDK tests with msw mocking"
```

---

### Task 8: Lint, create PR for TypeScript SDK

**Step 1: Run checks**

```bash
cd sdks/typescript && npm run lint && npm test
```

**Step 2: Create PR**

```bash
git checkout -b feat/typescript-sdk
git push -u origin feat/typescript-sdk
gh pr create --title "feat(sdk): TypeScript SDK with REST client" \
  --body "Adds sdks/typescript/ (@chorus/sdk) — zero-dep REST client using native fetch." \
  --label "enhancement" --assignee "MrBT-nano"
```

---

## PR 3: Java SDK

### Task 9: Scaffold Java SDK project

**Files:**
- Create: `sdks/java/pom.xml`
- Create: `sdks/java/.gitignore`
- Create: directory structure

**Step 1: Create directories**

```bash
mkdir -p sdks/java/src/main/java/com/chorus/sdk/model
mkdir -p sdks/java/src/main/java/com/chorus/sdk/exception
mkdir -p sdks/java/src/test/java/com/chorus/sdk
```

**Step 2: Create pom.xml**

In `sdks/java/pom.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0
                             http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <groupId>com.chorus</groupId>
    <artifactId>chorus-sdk</artifactId>
    <version>0.2.0</version>
    <packaging>jar</packaging>

    <name>Chorus SDK</name>
    <description>Official Java SDK for Chorus CPaaS — SMS, Email, OTP</description>
    <url>https://github.com/cntm-labs/chorus</url>

    <licenses>
        <license>
            <name>MIT</name>
            <url>https://opensource.org/licenses/MIT</url>
        </license>
    </licenses>

    <properties>
        <maven.compiler.source>11</maven.compiler.source>
        <maven.compiler.target>11</maven.compiler.target>
        <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
        <jackson.version>2.17.0</jackson.version>
    </properties>

    <dependencies>
        <dependency>
            <groupId>com.fasterxml.jackson.core</groupId>
            <artifactId>jackson-databind</artifactId>
            <version>${jackson.version}</version>
        </dependency>

        <!-- Test -->
        <dependency>
            <groupId>org.junit.jupiter</groupId>
            <artifactId>junit-jupiter</artifactId>
            <version>5.10.2</version>
            <scope>test</scope>
        </dependency>
        <dependency>
            <groupId>org.wiremock</groupId>
            <artifactId>wiremock</artifactId>
            <version>3.5.4</version>
            <scope>test</scope>
        </dependency>
    </dependencies>

    <build>
        <plugins>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-surefire-plugin</artifactId>
                <version>3.2.5</version>
            </plugin>
        </plugins>
    </build>
</project>
```

**Step 3: Create .gitignore**

In `sdks/java/.gitignore`:

```
target/
*.class
.idea/
*.iml
```

**Step 4: Commit**

```bash
git add sdks/java/
git commit -m "chore(sdk): scaffold Java SDK Maven project"
```

---

### Task 10: Implement Java SDK model classes

**Files:**
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/SendSmsRequest.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/SendEmailRequest.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/SendResponse.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/BatchSendRequest.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/BatchSendResponse.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/OtpSendRequest.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/OtpVerifyRequest.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/CreateWebhookRequest.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/model/WebhookResponse.java`
- Create: `sdks/java/src/main/java/com/chorus/sdk/exception/ChorusException.java`

**Step 1: Create core model POJOs**

In `SendSmsRequest.java`:
```java
package com.chorus.sdk.model;

import com.fasterxml.jackson.annotation.JsonInclude;

@JsonInclude(JsonInclude.Include.NON_NULL)
public class SendSmsRequest {
    private String to;
    private String body;
    private String from;

    public SendSmsRequest() {}

    public SendSmsRequest(String to, String body) {
        this.to = to;
        this.body = body;
    }

    public String getTo() { return to; }
    public void setTo(String to) { this.to = to; }
    public String getBody() { return body; }
    public void setBody(String body) { this.body = body; }
    public String getFrom() { return from; }
    public void setFrom(String from) { this.from = from; }
}
```

In `SendEmailRequest.java`:
```java
package com.chorus.sdk.model;

import com.fasterxml.jackson.annotation.JsonInclude;

@JsonInclude(JsonInclude.Include.NON_NULL)
public class SendEmailRequest {
    private String to;
    private String subject;
    private String body;
    private String from;

    public SendEmailRequest() {}

    public SendEmailRequest(String to, String subject, String body) {
        this.to = to;
        this.subject = subject;
        this.body = body;
    }

    public String getTo() { return to; }
    public void setTo(String to) { this.to = to; }
    public String getSubject() { return subject; }
    public void setSubject(String subject) { this.subject = subject; }
    public String getBody() { return body; }
    public void setBody(String body) { this.body = body; }
    public String getFrom() { return from; }
    public void setFrom(String from) { this.from = from; }
}
```

In `SendResponse.java`:
```java
package com.chorus.sdk.model;

import com.fasterxml.jackson.annotation.JsonProperty;

public class SendResponse {
    @JsonProperty("message_id")
    private String messageId;
    private String status;

    public String getMessageId() { return messageId; }
    public void setMessageId(String messageId) { this.messageId = messageId; }
    public String getStatus() { return status; }
    public void setStatus(String status) { this.status = status; }
}
```

In `BatchSendRequest.java`:
```java
package com.chorus.sdk.model;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.List;
import java.util.Map;

@JsonInclude(JsonInclude.Include.NON_NULL)
public class BatchSendRequest {
    private List<Map<String, String>> recipients;
    private String from;

    public BatchSendRequest() {}

    public BatchSendRequest(List<Map<String, String>> recipients) {
        this.recipients = recipients;
    }

    public List<Map<String, String>> getRecipients() { return recipients; }
    public void setRecipients(List<Map<String, String>> recipients) { this.recipients = recipients; }
    public String getFrom() { return from; }
    public void setFrom(String from) { this.from = from; }
}
```

In `BatchSendResponse.java`:
```java
package com.chorus.sdk.model;

import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.List;

public class BatchSendResponse {
    private List<BatchMessage> messages;
    private String error;

    public List<BatchMessage> getMessages() { return messages; }
    public void setMessages(List<BatchMessage> messages) { this.messages = messages; }
    public String getError() { return error; }
    public void setError(String error) { this.error = error; }

    public static class BatchMessage {
        @JsonProperty("message_id")
        private String messageId;
        private String to;
        private String status;

        public String getMessageId() { return messageId; }
        public void setMessageId(String messageId) { this.messageId = messageId; }
        public String getTo() { return to; }
        public void setTo(String to) { this.to = to; }
        public String getStatus() { return status; }
        public void setStatus(String status) { this.status = status; }
    }
}
```

In `OtpSendRequest.java`:
```java
package com.chorus.sdk.model;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;

@JsonInclude(JsonInclude.Include.NON_NULL)
public class OtpSendRequest {
    private String to;
    @JsonProperty("app_name")
    private String appName;

    public OtpSendRequest() {}

    public OtpSendRequest(String to, String appName) {
        this.to = to;
        this.appName = appName;
    }

    public String getTo() { return to; }
    public void setTo(String to) { this.to = to; }
    public String getAppName() { return appName; }
    public void setAppName(String appName) { this.appName = appName; }
}
```

In `OtpVerifyRequest.java`:
```java
package com.chorus.sdk.model;

public class OtpVerifyRequest {
    private String to;
    private String code;

    public OtpVerifyRequest() {}

    public OtpVerifyRequest(String to, String code) {
        this.to = to;
        this.code = code;
    }

    public String getTo() { return to; }
    public void setTo(String to) { this.to = to; }
    public String getCode() { return code; }
    public void setCode(String code) { this.code = code; }
}
```

In `CreateWebhookRequest.java`:
```java
package com.chorus.sdk.model;

import java.util.List;

public class CreateWebhookRequest {
    private String url;
    private List<String> events;

    public CreateWebhookRequest() {}

    public CreateWebhookRequest(String url, List<String> events) {
        this.url = url;
        this.events = events;
    }

    public String getUrl() { return url; }
    public void setUrl(String url) { this.url = url; }
    public List<String> getEvents() { return events; }
    public void setEvents(List<String> events) { this.events = events; }
}
```

In `WebhookResponse.java`:
```java
package com.chorus.sdk.model;

import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.List;

public class WebhookResponse {
    private String id;
    private String url;
    private String secret;
    private List<String> events;
    @JsonProperty("created_at")
    private String createdAt;

    public String getId() { return id; }
    public void setId(String id) { this.id = id; }
    public String getUrl() { return url; }
    public void setUrl(String url) { this.url = url; }
    public String getSecret() { return secret; }
    public void setSecret(String secret) { this.secret = secret; }
    public List<String> getEvents() { return events; }
    public void setEvents(List<String> events) { this.events = events; }
    public String getCreatedAt() { return createdAt; }
    public void setCreatedAt(String createdAt) { this.createdAt = createdAt; }
}
```

In `ChorusException.java`:
```java
package com.chorus.sdk.exception;

/** Exception thrown when the Chorus API returns an error. */
public class ChorusException extends RuntimeException {
    private final int status;
    private final String body;

    public ChorusException(int status, String body) {
        super("Chorus API error (" + status + "): " + body);
        this.status = status;
        this.body = body;
    }

    public int getStatus() { return status; }
    public String getBody() { return body; }
}
```

**Step 2: Commit**

```bash
git add sdks/java/src/main/
git commit -m "feat(sdk): add Java SDK model classes and exception"
```

---

### Task 11: Implement Java SDK ChorusClient

**Files:**
- Create: `sdks/java/src/main/java/com/chorus/sdk/ChorusClient.java`

**Step 1: Create ChorusClient.java**

```java
package com.chorus.sdk;

import com.chorus.sdk.exception.ChorusException;
import com.chorus.sdk.model.*;
import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;

import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.List;

/** Chorus CPaaS client for Java. */
public class ChorusClient {
    private final String apiKey;
    private final String baseUrl;
    private final HttpClient httpClient;
    private final ObjectMapper mapper;

    private ChorusClient(Builder builder) {
        this.apiKey = builder.apiKey;
        this.baseUrl = builder.baseUrl.replaceAll("/+$", "");
        this.httpClient = HttpClient.newHttpClient();
        this.mapper = new ObjectMapper()
            .configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);
    }

    /** Create a builder for ChorusClient. */
    public static Builder builder() {
        return new Builder();
    }

    /** SMS operations. */
    public SmsOps sms() { return new SmsOps(); }

    /** Email operations. */
    public EmailOps email() { return new EmailOps(); }

    /** OTP operations. */
    public OtpOps otp() { return new OtpOps(); }

    /** Message queries. */
    public MessageOps messages() { return new MessageOps(); }

    /** Webhook management. */
    public WebhookOps webhooks() { return new WebhookOps(); }

    private <T> T post(String path, Object body, Class<T> type) {
        try {
            String json = mapper.writeValueAsString(body);
            HttpRequest req = HttpRequest.newBuilder()
                .uri(URI.create(baseUrl + path))
                .header("Authorization", "Bearer " + apiKey)
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(json))
                .build();
            HttpResponse<String> resp = httpClient.send(req, HttpResponse.BodyHandlers.ofString());
            if (resp.statusCode() >= 400) {
                throw new ChorusException(resp.statusCode(), resp.body());
            }
            return mapper.readValue(resp.body(), type);
        } catch (ChorusException e) {
            throw e;
        } catch (IOException | InterruptedException e) {
            throw new RuntimeException("Chorus API request failed", e);
        }
    }

    private <T> T get(String path, Class<T> type) {
        try {
            HttpRequest req = HttpRequest.newBuilder()
                .uri(URI.create(baseUrl + path))
                .header("Authorization", "Bearer " + apiKey)
                .GET()
                .build();
            HttpResponse<String> resp = httpClient.send(req, HttpResponse.BodyHandlers.ofString());
            if (resp.statusCode() >= 400) {
                throw new ChorusException(resp.statusCode(), resp.body());
            }
            return mapper.readValue(resp.body(), type);
        } catch (ChorusException e) {
            throw e;
        } catch (IOException | InterruptedException e) {
            throw new RuntimeException("Chorus API request failed", e);
        }
    }

    private <T> T get(String path, TypeReference<T> type) {
        try {
            HttpRequest req = HttpRequest.newBuilder()
                .uri(URI.create(baseUrl + path))
                .header("Authorization", "Bearer " + apiKey)
                .GET()
                .build();
            HttpResponse<String> resp = httpClient.send(req, HttpResponse.BodyHandlers.ofString());
            if (resp.statusCode() >= 400) {
                throw new ChorusException(resp.statusCode(), resp.body());
            }
            return mapper.readValue(resp.body(), type);
        } catch (ChorusException e) {
            throw e;
        } catch (IOException | InterruptedException e) {
            throw new RuntimeException("Chorus API request failed", e);
        }
    }

    private void delete(String path) {
        try {
            HttpRequest req = HttpRequest.newBuilder()
                .uri(URI.create(baseUrl + path))
                .header("Authorization", "Bearer " + apiKey)
                .DELETE()
                .build();
            HttpResponse<String> resp = httpClient.send(req, HttpResponse.BodyHandlers.ofString());
            if (resp.statusCode() >= 400) {
                throw new ChorusException(resp.statusCode(), resp.body());
            }
        } catch (ChorusException e) {
            throw e;
        } catch (IOException | InterruptedException e) {
            throw new RuntimeException("Chorus API request failed", e);
        }
    }

    public class SmsOps {
        public SendResponse send(SendSmsRequest req) { return post("/v1/sms/send", req, SendResponse.class); }
        public BatchSendResponse sendBatch(BatchSendRequest req) { return post("/v1/sms/send-batch", req, BatchSendResponse.class); }
    }

    public class EmailOps {
        public SendResponse send(SendEmailRequest req) { return post("/v1/email/send", req, SendResponse.class); }
        public BatchSendResponse sendBatch(BatchSendRequest req) { return post("/v1/email/send-batch", req, BatchSendResponse.class); }
    }

    public class OtpOps {
        public SendResponse send(OtpSendRequest req) { return post("/v1/otp/send", req, SendResponse.class); }
        public SendResponse verify(OtpVerifyRequest req) { return post("/v1/otp/verify", req, SendResponse.class); }
    }

    public class MessageOps {
        public SendResponse get(String id) { return ChorusClient.this.get("/v1/messages/" + id, SendResponse.class); }
        public List<SendResponse> list() { return ChorusClient.this.get("/v1/messages", new TypeReference<>() {}); }
    }

    public class WebhookOps {
        public WebhookResponse create(CreateWebhookRequest req) { return post("/v1/webhooks", req, WebhookResponse.class); }
        public List<WebhookResponse> list() { return ChorusClient.this.get("/v1/webhooks", new TypeReference<>() {}); }
        public void delete(String id) { ChorusClient.this.delete("/v1/webhooks/" + id); }
    }

    /** Builder for ChorusClient. */
    public static class Builder {
        private String apiKey;
        private String baseUrl = "http://localhost:3000";

        /** Set the API key. */
        public Builder apiKey(String apiKey) { this.apiKey = apiKey; return this; }

        /** Set the base URL. */
        public Builder baseUrl(String baseUrl) { this.baseUrl = baseUrl; return this; }

        /** Build the client. */
        public ChorusClient build() {
            if (apiKey == null || apiKey.isEmpty()) {
                throw new IllegalArgumentException("apiKey is required");
            }
            return new ChorusClient(this);
        }
    }
}
```

**Step 2: Compile**

```bash
cd sdks/java && mvn compile
```

**Step 3: Commit**

```bash
git add sdks/java/src/main/java/com/chorus/sdk/ChorusClient.java
git commit -m "feat(sdk): implement Java SDK ChorusClient with builder pattern"
```

---

### Task 12: Add Java SDK tests

**Files:**
- Create: `sdks/java/src/test/java/com/chorus/sdk/ChorusClientTest.java`

**Step 1: Create test**

```java
package com.chorus.sdk;

import com.chorus.sdk.exception.ChorusException;
import com.chorus.sdk.model.*;
import com.github.tomakehurst.wiremock.junit5.WireMockRuntimeInfo;
import com.github.tomakehurst.wiremock.junit5.WireMockTest;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

import static com.github.tomakehurst.wiremock.client.WireMock.*;
import static org.junit.jupiter.api.Assertions.*;

@WireMockTest
class ChorusClientTest {

    private ChorusClient buildClient(WireMockRuntimeInfo wm) {
        return ChorusClient.builder()
            .apiKey("ch_test_xxx")
            .baseUrl(wm.getHttpBaseUrl())
            .build();
    }

    @Test
    void sendSms(WireMockRuntimeInfo wm) {
        stubFor(post("/v1/sms/send").willReturn(
            okJson("{\"message_id\": \"msg-1\", \"status\": \"queued\"}")
        ));
        var client = buildClient(wm);
        var res = client.sms().send(new SendSmsRequest("+111", "hi"));
        assertEquals("msg-1", res.getMessageId());
        assertEquals("queued", res.getStatus());
    }

    @Test
    void sendEmail(WireMockRuntimeInfo wm) {
        stubFor(post("/v1/email/send").willReturn(
            okJson("{\"message_id\": \"msg-2\", \"status\": \"queued\"}")
        ));
        var client = buildClient(wm);
        var res = client.email().send(new SendEmailRequest("a@b.com", "Hi", "Hello"));
        assertEquals("msg-2", res.getMessageId());
    }

    @Test
    void createWebhook(WireMockRuntimeInfo wm) {
        stubFor(post("/v1/webhooks").willReturn(
            okJson("{\"id\": \"wh-1\", \"url\": \"https://example.com\", \"secret\": \"abc\", \"events\": [\"message.delivered\"], \"created_at\": \"2026-01-01\"}")
        ));
        var client = buildClient(wm);
        var wh = client.webhooks().create(new CreateWebhookRequest("https://example.com", List.of("message.delivered")));
        assertEquals("wh-1", wh.getId());
        assertEquals("abc", wh.getSecret());
    }

    @Test
    void apiErrorThrowsChorusException(WireMockRuntimeInfo wm) {
        stubFor(post("/v1/sms/send").willReturn(
            unauthorized().withBody("Invalid API key")
        ));
        var client = buildClient(wm);
        var ex = assertThrows(ChorusException.class, () ->
            client.sms().send(new SendSmsRequest("+111", "hi"))
        );
        assertEquals(401, ex.getStatus());
    }

    @Test
    void builderRequiresApiKey() {
        assertThrows(IllegalArgumentException.class, () ->
            ChorusClient.builder().build()
        );
    }
}
```

**Step 2: Run tests**

```bash
cd sdks/java && mvn test
```
Expected: All 5 tests PASS

**Step 3: Commit**

```bash
git add sdks/java/src/test/
git commit -m "test(sdk): add Java SDK tests with WireMock"
```

---

### Task 13: Create PR for Java SDK

```bash
git checkout -b feat/java-sdk
git push -u origin feat/java-sdk
gh pr create --title "feat(sdk): Java SDK with HttpClient and builder pattern" \
  --body "Adds sdks/java/ (com.chorus:chorus-sdk) — Java 11+ REST client using java.net.http." \
  --label "enhancement" --assignee "MrBT-nano"
```

---

## PR 4: README Update + Version Bump

### Task 14: Update README

**Files:**
- Modify: `README.md`

**Step 1: Update features section**

Change `6 providers` to `7 providers` (add Mailgun). Add webhooks and batch send to features list.

**Step 2: Update SDK section in architecture**

```
sdks/
├── rust/              # Native SDK (re-exports chorus-core)
├── typescript/        # Node.js + Browser
├── java/              # Java 11+
├── go/                # Coming soon
├── python/            # Coming soon
└── c/                 # Coming soon
```

**Step 3: Commit**

```bash
git add README.md
git commit -m "docs: update README for v0.2.0 — 7 providers, webhooks, batch, SDKs"
```

---

### Task 15: Bump version to 0.2.0

**Files:**
- Modify: `crates/chorus-core/Cargo.toml` (version = "0.2.0")
- Modify: `crates/chorus-providers/Cargo.toml` (version = "0.2.0", chorus-core dep)
- Modify: `crates/chorus-server/Cargo.toml` (version = "0.2.0", deps)
- Modify: `sdks/rust/Cargo.toml` (chorus-core dep version)

**Step 1: Bump all crate versions**

```bash
sed -i '' 's/version = "0.1.1"/version = "0.2.0"/' crates/*/Cargo.toml sdks/rust/Cargo.toml
sed -i '' 's/version = "0.1.1"/version = "0.2.0"/' crates/chorus-providers/Cargo.toml crates/chorus-server/Cargo.toml
```

Verify inter-crate dep versions also updated.

**Step 2: Run full CI**

```bash
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

**Step 3: Commit**

```bash
git add -A
git commit -m "chore: bump version to 0.2.0"
```

**Step 4: Create PR**

```bash
git checkout -b release/v0.2.0
git push -u origin release/v0.2.0
gh pr create --title "chore: v0.2.0 — Mailgun, webhooks, batch send, SDKs" \
  --body "Version bump to 0.2.0 with README update." \
  --assignee "MrBT-nano"
```
