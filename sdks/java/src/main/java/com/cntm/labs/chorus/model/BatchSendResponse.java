package com.cntm.labs.chorus.model;

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
