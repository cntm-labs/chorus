/**
 * @file chorus.h
 * @brief Chorus CPaaS C SDK — SMS, Email, OTP with smart routing.
 *
 * Requires libcurl for HTTP requests.
 */

#ifndef CHORUS_H
#define CHORUS_H

#ifdef __cplusplus
extern "C" {
#endif

/** Return codes. */
typedef enum {
    CHORUS_OK = 0,
    CHORUS_ERR_CURL = -1,
    CHORUS_ERR_HTTP = -2,
    CHORUS_ERR_JSON = -3,
    CHORUS_ERR_ALLOC = -4,
} chorus_status_t;

/** Opaque client handle. */
typedef struct chorus_client chorus_client_t;

/**
 * Create a new Chorus client.
 * @param api_key   Bearer token (e.g. "ch_live_...").
 * @param base_url  Server URL or NULL for "http://localhost:3000".
 * @return Client handle or NULL on allocation failure.
 */
chorus_client_t *chorus_client_new(const char *api_key, const char *base_url);

/** Free a client handle. */
void chorus_client_free(chorus_client_t *client);

/** Get the HTTP status code from the last request. */
int chorus_last_http_status(const chorus_client_t *client);

/** Get the raw response body from the last request (NULL-terminated). */
const char *chorus_last_response(const chorus_client_t *client);

/**
 * Send a single SMS.
 * @param out_message_id  Buffer to receive message ID (>= 64 bytes).
 * @return CHORUS_OK on success.
 */
chorus_status_t chorus_sms_send(
    chorus_client_t *client,
    const char *to,
    const char *body,
    const char *from,
    char *out_message_id,
    int out_len
);

/**
 * Send a single email.
 * @param out_message_id  Buffer to receive message ID (>= 64 bytes).
 * @return CHORUS_OK on success.
 */
chorus_status_t chorus_email_send(
    chorus_client_t *client,
    const char *to,
    const char *subject,
    const char *body,
    const char *from,
    char *out_message_id,
    int out_len
);

/**
 * Send an OTP code.
 * @param out_message_id  Buffer to receive message ID (>= 64 bytes).
 * @param out_expires_in  Receives expiry in seconds.
 * @return CHORUS_OK on success.
 */
chorus_status_t chorus_otp_send(
    chorus_client_t *client,
    const char *to,
    const char *app_name,
    char *out_message_id,
    int out_len,
    int *out_expires_in
);

/**
 * Verify an OTP code.
 * @param out_valid  Receives 1 if valid, 0 otherwise.
 * @return CHORUS_OK on success.
 */
chorus_status_t chorus_otp_verify(
    chorus_client_t *client,
    const char *to,
    const char *code,
    int *out_valid
);

#ifdef __cplusplus
}
#endif

#endif /* CHORUS_H */
