FROM python:3 as base

SHELL ["/bin/bash", "-c"]

ENV PIP_DEFAULT_TIMEOUT=100 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1

RUN pip install poetry

WORKDIR /src

COPY . .

RUN poetry install --no-root --no-interaction --no-ansi

# ====================================
FROM base as test

WORKDIR /src

COPY . .

CMD poetry run pytest -rP
