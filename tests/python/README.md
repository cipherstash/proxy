# Python integration tests

Language-integration tests that exercise CipherStash Proxy from Python clients.

From the repo root, the canonical entry point is:

```sh
mise run test:integration:lang:python
```

To run the suite manually:

Install pipx
https://pipx.pypa.io/stable/installation/

```
brew install pipx
pipx ensurepath
```


Install Poetry
https://python-poetry.org/docs/#installation

```
pipx install poetry
```


Install Dependencies

```
poetry install
```

Run Tests

```
poetry run pytest -rP
```

