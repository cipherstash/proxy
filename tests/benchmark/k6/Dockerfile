FROM golang:1.22-alpine AS builder
RUN apk add --no-cache git
RUN go install go.k6.io/xk6/cmd/xk6@latest
WORKDIR /build
RUN xk6 build --with github.com/gcfabri/xk6-pgxpool@latest --output /build/k6

FROM alpine:3.19
RUN apk add --no-cache ca-certificates
COPY --from=builder /build/k6 /usr/local/bin/k6
WORKDIR /scripts
ENTRYPOINT ["k6"]
