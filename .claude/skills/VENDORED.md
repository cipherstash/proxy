# Vendored skills

`quint-lang/` and `quint-modeling/` are copied unchanged from the Quint LLM Kit, so every session uses the same pinned version.

- Source: https://github.com/quint-co/quint-llm-kit, directory `quint-llm-kit-plugin/skills/`
- Commit: `cc75369f741af7d490936f82002c2d28e3b3d78d` (copied 2026-10-26)
- Licence: Apache-2.0, copied into each skill directory as `LICENSE`

To update, copy both directories from a newer commit, keep the `LICENSE` files, update the commit above, and rerun `specs/quint/check.sh all`.
Put Proxy-specific rules in `proxy-quint/`, not in the vendored files.
The kit's third skill, `quint-execute-spec`, is not vendored, so `quint-modeling`'s pointers to it lead nowhere here.
