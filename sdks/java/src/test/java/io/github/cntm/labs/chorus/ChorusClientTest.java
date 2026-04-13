package io.github.cntm.labs.chorus;

import io.github.cntm.labs.chorus.exception.ChorusException;
import io.github.cntm.labs.chorus.model.*;
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
