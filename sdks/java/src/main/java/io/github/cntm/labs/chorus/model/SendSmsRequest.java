package io.github.cntm.labs.chorus.model;

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
