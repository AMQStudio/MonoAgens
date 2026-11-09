# Commands
- Env: `uv venv .venv --python 3.14 && uv pip install --python .venv/bin/python -e ".[dev,whatsapp,telegram,discord,slack]"`.
- Test DB: `podman run -d --name monoagens-test-pg -p 5433:5432 -e POSTGRES_DB=monoagens_test -e POSTGRES_USER=monoagens -e POSTGRES_PASSWORD=monoagens docker.io/pgvector/pgvector:pg17`; export `MONOAGENS_TEST_DATABASE_URL=postgresql+psycopg://monoagens:monoagens@127.0.0.1:5433/monoagens_test`.
- Tests: `.venv/bin/pytest -q` (single: `.venv/bin/pytest -q tests/test_x.py -k name`).
- Run scripts with `-P`: `.venv/bin/python -P scripts/<name>.py --check`.
- CLI entry: `.venv/bin/python -m monoagens ...` (e.g. `plugins digest plugins/<name>`, `db upgrade`).
- New ADR: `.venv/bin/python -P scripts/adr.py --new "Title"`.
- Knowledge graph: `graphify query "<q>"`; refresh `/graphify . --update` (excludes in `.graphifyignore`).
