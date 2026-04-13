package com.cntm.labs.chorus.model;

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
