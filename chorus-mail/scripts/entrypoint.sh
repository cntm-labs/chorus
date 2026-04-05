#!/bin/bash
set -e

MAIL_DOMAIN="${MAIL_DOMAIN:?MAIL_DOMAIN is required}"
HOSTNAME="${MAIL_HOSTNAME:-mail.${MAIL_DOMAIN}}"

echo "chorus-mail: configuring for domain ${MAIL_DOMAIN}"

# Replace template variables in Postfix config
sed -e "s/__MAIL_DOMAIN__/${MAIL_DOMAIN}/g" \
    -e "s/__HOSTNAME__/${HOSTNAME}/g" \
    /etc/postfix/main.cf.template > /etc/postfix/main.cf

# Replace template variables in OpenDKIM config
sed "s/__MAIL_DOMAIN__/${MAIL_DOMAIN}/g" \
    /etc/opendkim/opendkim.conf.template > /etc/opendkim/opendkim.conf

# Generate DKIM keys if they don't exist
DKIM_DIR="/etc/opendkim/keys/${MAIL_DOMAIN}"
if [ ! -f "${DKIM_DIR}/chorus.private" ]; then
    echo "chorus-mail: generating DKIM keys for ${MAIL_DOMAIN}"
    mkdir -p "${DKIM_DIR}"
    opendkim-genkey -b 2048 -d "${MAIL_DOMAIN}" -D "${DKIM_DIR}" -s chorus -v
    chown -R opendkim:opendkim /etc/opendkim/keys
fi

# Create OpenDKIM run directory
mkdir -p /run/opendkim
chown opendkim:opendkim /run/opendkim

# Start OpenDKIM
opendkim -x /etc/opendkim/opendkim.conf &

# Start Postfix in foreground
echo "chorus-mail: starting Postfix for ${MAIL_DOMAIN}"
postfix start-fg
