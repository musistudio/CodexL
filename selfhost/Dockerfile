FROM denoland/deno:alpine

WORKDIR /app

COPY public ./public
COPY selfhost/relay.js ./selfhost/relay.js

ENV HOST=0.0.0.0
ENV PORT=8443
ENV TLS_CERT=/certs/fullchain.pem
ENV TLS_KEY=/certs/privkey.pem

EXPOSE 8443/tcp
EXPOSE 8443/udp

CMD ["run", "--allow-net", "--allow-read", "--allow-env", "--unstable-net", "selfhost/relay.js"]
