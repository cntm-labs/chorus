package com.cntm.labs.chorus;

import com.cntm.labs.chorus.exception.ChorusException;
import com.cntm.labs.chorus.model.*;
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
