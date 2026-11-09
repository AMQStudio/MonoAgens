# Task completion gate
Run all, zero errors:
1. `uv lock --check` (run `uv lock` first if pyproject deps changed)
2. `.venv/bin/ruff check monoagens scripts tests plugins migrations`
3. `.venv/bin/python -P scripts/selfcheck.py --check`
4. `.venv/bin/python -P scripts/types.py --check`
5. `.venv/bin/python -P scripts/adr.py --check`
6. `.venv/bin/pytest -q` (with `MONOAGENS_TEST_DATABASE_URL` set)

Sync before gate:
- Config dataclasses (`engine/config/`, `engine/soul.py`, `engine/access/roles.py`) or `admin/templates.py` changed -> regenerate root `config.yaml`, `roles.yaml`, `soul.yaml`.
- `plugins/<name>/` changed -> `.venv/bin/python -m monoagens plugins digest plugins/<name>` and update `digest:`.
- Schema change -> new Alembic revision in linear chain.
- After code changes -> `/graphify . --update`.
