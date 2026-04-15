#include "chorus.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

static void test_client_create_and_free(void) {
    chorus_client_t *c = chorus_client_new("ch_test_xxx", NULL);
    assert(c != NULL);
    chorus_client_free(c);
    printf("  PASS: client create and free\n");
}

static void test_client_with_custom_url(void) {
    chorus_client_t *c = chorus_client_new("ch_test_xxx", "http://localhost:9999");
    assert(c != NULL);
    assert(chorus_last_http_status(c) == 0);
    assert(chorus_last_response(c) == NULL);
    chorus_client_free(c);
    printf("  PASS: client with custom URL\n");
}

static void test_null_client_accessors(void) {
    assert(chorus_last_http_status(NULL) == 0);
    assert(chorus_last_response(NULL) == NULL);
    printf("  PASS: null client accessors\n");
}

static void test_sms_send_connection_refused(void) {
    /* No server running — should return CHORUS_ERR_CURL */
    chorus_client_t *c = chorus_client_new("ch_test_xxx", "http://127.0.0.1:1");
    char msg_id[64] = {0};
    chorus_status_t st = chorus_sms_send(c, "+111", "hi", NULL, msg_id, sizeof(msg_id));
    assert(st == CHORUS_ERR_CURL);
    chorus_client_free(c);
    printf("  PASS: sms send connection refused\n");
}

int main(void) {
    printf("Running C SDK tests...\n");
    test_client_create_and_free();
    test_client_with_custom_url();
    test_null_client_accessors();
    test_sms_send_connection_refused();
    printf("All tests passed!\n");
    return 0;
}
