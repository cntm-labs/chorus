#!/bin/bash
set -e

MAIL_DOMAIN="${MAIL_DOMAIN:?MAIL_DOMAIN is required}"
DKIM_KEY_FILE="/etc/opendkim/keys/${MAIL_DOMAIN}/chorus.txt"

echo "═══════════════════════════════════════════════════"
echo "  DNS Records for: ${MAIL_DOMAIN}"
echo "═══════════════════════════════════════════════════"
echo ""
echo "Add these records to your DNS provider:"
echo ""
echo "─── MX Record ────────────────────────────────────"
echo "  Type:  MX"
echo "  Name:  @"
echo "  Value: mail.${MAIL_DOMAIN}"
echo "  Priority: 10"
echo ""
echo "─── A Record ─────────────────────────────────────"
echo "  Type:  A"
echo "  Name:  mail"
echo "  Value: <YOUR_SERVER_IP>"
echo ""
echo "─── SPF Record ───────────────────────────────────"
echo "  Type:  TXT"
echo "  Name:  @"
echo "  Value: \"v=spf1 a mx ip4:<YOUR_SERVER_IP> -all\""
echo ""
echo "─── DKIM Record ──────────────────────────────────"
echo "  Type:  TXT"
echo "  Name:  chorus._domainkey"
if [ -f "${DKIM_KEY_FILE}" ]; then
    DKIM_VALUE=$(grep -o '".*"' "${DKIM_KEY_FILE}" | tr -d '\n' | sed 's/" "//g')
    echo "  Value: ${DKIM_VALUE}"
else
    echo "  Value: <run entrypoint first to generate DKIM keys>"
fi
echo ""
echo "─── DMARC Record ─────────────────────────────────"
echo "  Type:  TXT"
echo "  Name:  _dmarc"
echo "  Value: \"v=DMARC1; p=quarantine; rua=mailto:postmaster@${MAIL_DOMAIN}\""
echo ""
echo "═══════════════════════════════════════════════════"
