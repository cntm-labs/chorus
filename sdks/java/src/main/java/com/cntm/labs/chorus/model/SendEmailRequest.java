package com.cntm.labs.chorus.model;

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
