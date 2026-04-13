package io.github.cntm.labs.chorus.model;

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
