FROM alpine:3.19

RUN apk add pgbouncer
RUN adduser -D -S pgbouncer

ADD pgbouncer.ini /etc/pgbouncer/pgbouncer.ini
ADD userlist.txt /etc/pgbouncer/userlist.txt

EXPOSE 6433
USER pgbouncer
CMD /usr/bin/pgbouncer /etc/pgbouncer/pgbouncer.ini
