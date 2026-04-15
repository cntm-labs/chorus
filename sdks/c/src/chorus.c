#include "chorus.h"
#include "../vendor/cJSON.h"

#include <curl/curl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DEFAULT_BASE_URL "http://localhost:3000"

struct chorus_client {
    char *api_key;
    char *base_url;
    CURL *curl;
    int last_http_status;
    char *last_response;
    size_t last_response_len;
};

/* curl write callback */
static size_t write_cb(void *data, size_t size, size_t nmemb, void *userp) {
    size_t total = size * nmemb;
    chorus_client_t *c = (chorus_client_t *)userp;
    char *tmp = realloc(c->last_response, c->last_response_len + total + 1);
    if (!tmp) return 0;
    c->last_response = tmp;
    memcpy(c->last_response + c->last_response_len, data, total);
    c->last_response_len += total;
    c->last_response[c->last_response_len] = '\0';
    return total;
}

chorus_client_t *chorus_client_new(const char *api_key, const char *base_url) {
    chorus_client_t *c = calloc(1, sizeof(*c));
    if (!c) return NULL;

    c->api_key = strdup(api_key);
    c->base_url = strdup(base_url ? base_url : DEFAULT_BASE_URL);
    c->curl = curl_easy_init();

    if (!c->api_key || !c->base_url || !c->curl) {
        chorus_client_free(c);
        return NULL;
    }
    return c;
}

void chorus_client_free(chorus_client_t *c) {
    if (!c) return;
    free(c->api_key);
    free(c->base_url);
    free(c->last_response);
    if (c->curl) curl_easy_cleanup(c->curl);
    free(c);
}

int chorus_last_http_status(const chorus_client_t *c) {
    return c ? c->last_http_status : 0;
}

const char *chorus_last_response(const chorus_client_t *c) {
    return c ? c->last_response : NULL;
}

/* Internal: perform a POST request with JSON body, parse response. */
static chorus_status_t do_post(chorus_client_t *c, const char *path, const char *json_body) {
    /* Reset response buffer */
    free(c->last_response);
    c->last_response = NULL;
    c->last_response_len = 0;
    c->last_http_status = 0;

    /* Build URL */
    size_t url_len = strlen(c->base_url) + strlen(path) + 1;
    char *url = malloc(url_len);
    if (!url) return CHORUS_ERR_ALLOC;
    snprintf(url, url_len, "%s%s", c->base_url, path);

    /* Build auth header */
    size_t auth_len = strlen("Authorization: Bearer ") + strlen(c->api_key) + 1;
    char *auth = malloc(auth_len);
    if (!auth) { free(url); return CHORUS_ERR_ALLOC; }
    snprintf(auth, auth_len, "Authorization: Bearer %s", c->api_key);

    struct curl_slist *headers = NULL;
    headers = curl_slist_append(headers, auth);
    headers = curl_slist_append(headers, "Content-Type: application/json");

    curl_easy_reset(c->curl);
    curl_easy_setopt(c->curl, CURLOPT_URL, url);
    curl_easy_setopt(c->curl, CURLOPT_HTTPHEADER, headers);
    curl_easy_setopt(c->curl, CURLOPT_POSTFIELDS, json_body);
    curl_easy_setopt(c->curl, CURLOPT_WRITEFUNCTION, write_cb);
    curl_easy_setopt(c->curl, CURLOPT_WRITEDATA, c);

    CURLcode res = curl_easy_perform(c->curl);
    curl_slist_free_all(headers);
    free(url);
    free(auth);

    if (res != CURLE_OK) return CHORUS_ERR_CURL;

    long http_code = 0;
    curl_easy_getinfo(c->curl, CURLINFO_RESPONSE_CODE, &http_code);
    c->last_http_status = (int)http_code;

    if (http_code >= 400) return CHORUS_ERR_HTTP;
    return CHORUS_OK;
}

/* Extract a string field from the last JSON response. */
static chorus_status_t extract_string(const chorus_client_t *c, const char *field, char *out, int out_len) {
    if (!c->last_response) return CHORUS_ERR_JSON;
    cJSON *root = cJSON_Parse(c->last_response);
    if (!root) return CHORUS_ERR_JSON;
    cJSON *item = cJSON_GetObjectItemCaseSensitive(root, field);
    if (!cJSON_IsString(item)) { cJSON_Delete(root); return CHORUS_ERR_JSON; }
    snprintf(out, out_len, "%s", item->valuestring);
    cJSON_Delete(root);
    return CHORUS_OK;
}

/* Extract an int field from the last JSON response. */
static chorus_status_t extract_int(const chorus_client_t *c, const char *field, int *out) {
    if (!c->last_response) return CHORUS_ERR_JSON;
    cJSON *root = cJSON_Parse(c->last_response);
    if (!root) return CHORUS_ERR_JSON;
    cJSON *item = cJSON_GetObjectItemCaseSensitive(root, field);
    if (!cJSON_IsNumber(item)) { cJSON_Delete(root); return CHORUS_ERR_JSON; }
    *out = item->valueint;
    cJSON_Delete(root);
    return CHORUS_OK;
}

/* Extract a bool field from the last JSON response. */
static chorus_status_t extract_bool(const chorus_client_t *c, const char *field, int *out) {
    if (!c->last_response) return CHORUS_ERR_JSON;
    cJSON *root = cJSON_Parse(c->last_response);
    if (!root) return CHORUS_ERR_JSON;
    cJSON *item = cJSON_GetObjectItemCaseSensitive(root, field);
    if (!cJSON_IsBool(item)) { cJSON_Delete(root); return CHORUS_ERR_JSON; }
    *out = cJSON_IsTrue(item) ? 1 : 0;
    cJSON_Delete(root);
    return CHORUS_OK;
}

chorus_status_t chorus_sms_send(
    chorus_client_t *c, const char *to, const char *body,
    const char *from, char *out_message_id, int out_len
) {
    cJSON *root = cJSON_CreateObject();
    cJSON_AddStringToObject(root, "to", to);
    cJSON_AddStringToObject(root, "body", body);
    if (from) cJSON_AddStringToObject(root, "from", from);
    char *json = cJSON_PrintUnformatted(root);
    cJSON_Delete(root);

    chorus_status_t st = do_post(c, "/v1/sms/send", json);
    free(json);
    if (st != CHORUS_OK) return st;
    return extract_string(c, "message_id", out_message_id, out_len);
}

chorus_status_t chorus_email_send(
    chorus_client_t *c, const char *to, const char *subject,
    const char *body, const char *from, char *out_message_id, int out_len
) {
    cJSON *root = cJSON_CreateObject();
    cJSON_AddStringToObject(root, "to", to);
    cJSON_AddStringToObject(root, "subject", subject);
    cJSON_AddStringToObject(root, "body", body);
    if (from) cJSON_AddStringToObject(root, "from", from);
    char *json = cJSON_PrintUnformatted(root);
    cJSON_Delete(root);

    chorus_status_t st = do_post(c, "/v1/email/send", json);
    free(json);
    if (st != CHORUS_OK) return st;
    return extract_string(c, "message_id", out_message_id, out_len);
}

chorus_status_t chorus_otp_send(
    chorus_client_t *c, const char *to, const char *app_name,
    char *out_message_id, int out_len, int *out_expires_in
) {
    cJSON *root = cJSON_CreateObject();
    cJSON_AddStringToObject(root, "to", to);
    if (app_name) cJSON_AddStringToObject(root, "app_name", app_name);
    char *json = cJSON_PrintUnformatted(root);
    cJSON_Delete(root);

    chorus_status_t st = do_post(c, "/v1/otp/send", json);
    free(json);
    if (st != CHORUS_OK) return st;

    st = extract_string(c, "message_id", out_message_id, out_len);
    if (st != CHORUS_OK) return st;
    return extract_int(c, "expires_in", out_expires_in);
}

chorus_status_t chorus_otp_verify(
    chorus_client_t *c, const char *to, const char *code, int *out_valid
) {
    cJSON *root = cJSON_CreateObject();
    cJSON_AddStringToObject(root, "to", to);
    cJSON_AddStringToObject(root, "code", code);
    char *json = cJSON_PrintUnformatted(root);
    cJSON_Delete(root);

    chorus_status_t st = do_post(c, "/v1/otp/verify", json);
    free(json);
    if (st != CHORUS_OK) return st;
    return extract_bool(c, "valid", out_valid);
}
