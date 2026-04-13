package com.cntm.labs.chorus.model;

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
