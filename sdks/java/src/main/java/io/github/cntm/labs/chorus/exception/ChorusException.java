package io.github.cntm.labs.chorus.exception;

/** Exception thrown when the Chorus API returns an error. */
public class ChorusException extends RuntimeException {
    private final int status;
    private final String body;

    public ChorusException(int status, String body) {
        super("Chorus API error (" + status + "): " + body);
        this.status = status;
        this.body = body;
    }

    public int getStatus() { return status; }
    public String getBody() { return body; }
}
