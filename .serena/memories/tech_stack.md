# Tech stack
- Python >=3.14, asyncio; packaging setuptools (PEP 639), package manager `uv` (`uv.lock` committed, `uv sync --locked` in install.sh/Dockerfile).
- DB: PostgreSQL 17 + pgvector (vector(384) HNSW) + pg_trgm; SQLAlchemy 2 + psycopg 3 + psycopg-pool; Alembic.
- LLM: `openai` SDK (OpenAI-compatible wire) + provider adapters; `mcp` client; `httpx`.
- NLP: sentence-transformers (embeddings), presidio-analyzer + spaCy `xx_ent_wiki_sm` (pinned wheel sha256), torch from pytorch-cpu index via `[tool.uv.sources]`.
- Gateway extras: whatsapp (neonize, segno), telegram (httpx Bot API), discord (discord.py), slack (slack-bolt/sdk).
- Dev: pytest, pytest-cov, ruff (line-length 100, py314), pyright.
- Tests need real Postgres+pgvector; no SQLite mocks.
