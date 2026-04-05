#!/bin/bash
# Postfix pipe transport: receives bounce notifications and forwards to chorus-server.
# Called by master.cf with: ${sender} ${recipient} ${original_recipient}

CHORUS_SERVER_URL="${CHORUS_SERVER_URL:?CHORUS_SERVER_URL is required}"
BOUNCE_SECRET="${BOUNCE_SECRET:?BOUNCE_SECRET is required}"

SENDER="$1"
RECIPIENT="$2"
ORIGINAL_RECIPIENT="$3"

# Read the bounce message from stdin
BOUNCE_BODY=$(cat)

# Extract the original Message-ID from bounce body if present
MESSAGE_ID=$(echo "${BOUNCE_BODY}" | grep -i "^Message-ID:" | head -1 | sed 's/Message-ID: *//i' | tr -d '<>' || echo "")

# Extract bounce reason from first diagnostic line
REASON=$(echo "${BOUNCE_BODY}" | grep -i "diagnostic-code:" | head -1 | sed 's/.*Diagnostic-Code: *//i' || echo "unknown bounce")

# POST to chorus-server
curl -sf -X POST "${CHORUS_SERVER_URL}/internal/bounces" \
    -H "Content-Type: application/json" \
    -H "X-Chorus-Secret: ${BOUNCE_SECRET}" \
    -d "{\"recipient\": \"${RECIPIENT}\", \"reason\": \"${REASON}\", \"message_id\": \"${MESSAGE_ID}\"}" \
    || echo "chorus-mail: failed to notify bounce for ${RECIPIENT}" >&2

exit 0
