package com.cntm.labs.chorus.model;

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
