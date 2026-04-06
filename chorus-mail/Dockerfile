FROM alpine:3.21

RUN apk add --no-cache \
    postfix \
    opendkim \
    opendkim-utils \
    curl \
    bash

COPY config/main.cf /etc/postfix/main.cf.template
COPY config/master.cf /etc/postfix/master.cf
COPY config/opendkim.conf /etc/opendkim/opendkim.conf.template
COPY scripts/ /scripts/
RUN chmod +x /scripts/*.sh

EXPOSE 25 587

ENTRYPOINT ["/scripts/entrypoint.sh"]
