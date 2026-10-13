ARG BUILD_FROM
FROM ${BUILD_FROM}

RUN apk add --no-cache python3

COPY server.py /server.py
CMD ["python3", "-u", "/server.py"]
