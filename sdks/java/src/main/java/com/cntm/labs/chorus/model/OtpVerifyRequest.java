package com.cntm.labs.chorus.model;

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
