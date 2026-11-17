# Run 4 Reference Note — R-55 Verbatim Prompts & Published Tables

Pinned primary sources verified `2026-11-18`:
- **Mem0 Paper (`arXiv:2504.19413v1`):** `https://arxiv.org/abs/2504.19413` (`https://arxiv.org/html/2504.19413v1`)
- **Mem0 Paper Evaluation Code (`mem0ai/mem0` @ `aae5989e78a6188b3b047c104d960c9ad0927e75`):** `https://github.com/mem0ai/mem0/tree/aae5989e78a6188b3b047c104d960c9ad0927e75/evaluation`
- **Mem0 Current Benchmark Suite (`mem0ai/memory-benchmarks` @ `4b61c5d31b9c668a12b4f5e78064248a02c82d2b`):** `https://github.com/mem0ai/memory-benchmarks/tree/4b61c5d31b9c668a12b4f5e78064248a02c82d2b`
- **Zep Paper (`arXiv:2501.13956v1`):** `https://arxiv.org/abs/2501.13956` (`https://arxiv.org/html/2501.13956v1`)
- **Zep Papers Code & LoCoMo Eval (`getzep/zep-papers` @ `4b7f26cc76cca20743314ba9acb8c2cb6adc42f6`):** `https://github.com/getzep/zep-papers/tree/4b7f26cc76cca20743314ba9acb8c2cb6adc42f6/kg_architecture_agent_memory`
- **Zep Blog Post (May 6, 2025, updated Jun 3, 2026):** `https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/`
- **Letta Blog Post (Aug 12, 2025):** `https://www.letta.com/blog/benchmarking-ai-agent-memory`
- **Letta Leaderboard Code (`letta-ai/letta-leaderboard` @ `802a794263279839f9384bfd518ca9da3d059d37`):** `https://github.com/letta-ai/letta-leaderboard/tree/802a794263279839f9384bfd518ca9da3d059d37`
- **LongMemEval Official Repo (`xiaowu0162/LongMemEval` @ `9e0b455f4ef0e2ab8f2e582289761153549043fc`):** `https://github.com/xiaowu0162/LongMemEval/tree/9e0b455f4ef0e2ab8f2e582289761153549043fc`

---

## 1. LoCoMo — Verbatim Prompts (Generation 1: April 2025 Paper Standard)

### 1.1 Mem0 Paper Answer Prompt (`evaluation/prompts.py`, `ANSWER_PROMPT`)

Used for `Mem0`, `LangMem`, `OpenAI` (with single `{memories}` block), and `Zep` (`ANSWER_PROMPT_ZEP`, same instructions with single `{memories}` block). `ANSWER_PROMPT_GRAPH` adds instruction #9 (`Use the knowledge graph relations...`) and `Relations for user {{speaker_*_user_id}}` blocks.

```text
    You are an intelligent memory assistant tasked with retrieving accurate information from conversation memories.

    # CONTEXT:
    You have access to memories from two speakers in a conversation. These memories contain 
    timestamped information that may be relevant to answering the question.

    # INSTRUCTIONS:
    1. Carefully analyze all provided memories from both speakers
    2. Pay special attention to the timestamps to determine the answer
    3. If the question asks about a specific event or fact, look for direct evidence in the memories
    4. If the memories contain contradictory information, prioritize the most recent memory
    5. If there is a question about time references (like "last year", "two months ago", etc.), 
       calculate the actual date based on the memory timestamp. For example, if a memory from 
       4 May 2022 mentions "went to India last year," then the trip occurred in 2021.
    6. Always convert relative time references to specific dates, months, or years. For example, 
       convert "last year" to "2022" or "two months ago" to "March 2023" based on the memory 
       timestamp. Ignore the reference while answering the question.
    7. Focus only on the content of the memories from both speakers. Do not confuse character 
       names mentioned in memories with the actual users who created those memories.
    8. The answer should be less than 5-6 words.

    # APPROACH (Think step by step):
    1. First, examine all memories that contain information related to the question
    2. Examine the timestamps and content of these memories carefully
    3. Look for explicit mentions of dates, times, locations, or events that answer the question
    4. If the answer requires calculation (e.g., converting relative time references), show your work
    5. Formulate a precise, concise answer based solely on the evidence in the memories
    6. Double-check that your answer directly addresses the question asked
    7. Ensure your final answer is specific and avoids vague time references

    Memories for user {{speaker_1_user_id}}:

    {{speaker_1_memories}}

    Memories for user {{speaker_2_user_id}}:

    {{speaker_2_memories}}

    Question: {{question}}

    Answer:
```

### 1.2 Mem0 Paper & Zep LoCoMo Judge Prompt (`evaluation/metrics/llm_judge.py`, `ACCURACY_PROMPT`)

Called with `model="gpt-4o-mini"`, `temperature=0.0`, `response_format={"type": "json_object"}` in Mem0 (`mem0ai/mem0` @ `aae5989e`) and `model="gpt-4o-mini"`, `temperature=0`, `response_format=Grade` in Zep (`getzep/zep-papers` @ `4b7f26cc`, `zep_locomo_eval.py`). Both skip Category 5 (`category == 5`).

```text
Your task is to label an answer to a question as ’CORRECT’ or ’WRONG’. You will be given the following data:
    (1) a question (posed by one user to another user), 
    (2) a ’gold’ (ground truth) answer, 
    (3) a generated answer
which you will score as CORRECT/WRONG.

The point of the question is to ask about something one user should know about the other user based on their prior conversations.
The gold answer will usually be a concise and short answer that includes the referenced topic, for example:
Question: Do you remember what I got the last time I went to Hawaii?
Gold answer: A shell necklace
The generated answer might be much longer, but you should be generous with your grading - as long as it touches on the same topic as the gold answer, it should be counted as CORRECT. 

For time related questions, the gold answer will be a specific date, month, year, etc. The generated answer might be much longer or use relative time references (like "last Tuesday" or "next month"), but you should be generous with your grading - as long as it refers to the same date or time period as the gold answer, it should be counted as CORRECT. Even if the format differs (e.g., "May 7th" vs "7 May"), consider it CORRECT if it's the same date.

Now it's time for the real question:
Question: {question}
Gold answer: {gold_answer}
Generated answer: {generated_answer}

First, provide a short (one sentence) explanation of your reasoning, then finish with CORRECT or WRONG. 
Do NOT include both CORRECT and WRONG in your response, or it will break the evaluation script.

Just return the label CORRECT or WRONG in a json format with the key as "label".
```

### 1.3 Zep LoCoMo Answer Prompt (`getzep/zep-papers` @ `4b7f26cc`, `zep_locomo_responses.py`)

System message: `"You are a helpful expert assistant answering questions from lme_experiment users based on the provided context."`
User prompt (`model="gpt-4o-mini"`, `temperature=0`):

```text
    # CONTEXT:
    You have access to facts and entities from a conversation.

    # INSTRUCTIONS:
    1. Carefully analyze all provided memories
    2. Pay special attention to the timestamps to determine the answer
    3. If the question asks about a specific event or fact, look for direct evidence in the memories
    4. If the memories contain contradictory information, prioritize the most recent memory
    5. Always convert relative time references to specific dates, months, or years.
    6. Be as specific as possible when talking about people, places, and events
    7. Timestamps in memories represent the actual time the event occurred, not the time the event was mentioned in a message.
    
    Clarification:
    When interpreting memories, use the timestamp to determine when the described event happened, not when someone talked about the event.
    
    Example:
    
    Memory: (2023-03-15T16:33:00Z) I went to the vet yesterday.
    Question: What day did I go to the vet?
    Correct Answer: March 15, 2023
    Explanation:
    Even though the phrase says "yesterday," the timestamp shows the event was recorded as happening on March 15th. Therefore, the actual vet visit happened on that date, regardless of the word "yesterday" in the text.


    # APPROACH (Think step by step):
    1. First, examine all memories that contain information related to the question
    2. Examine the timestamps and content of these memories carefully
    3. Look for explicit mentions of dates, times, locations, or events that answer the question
    4. If the answer requires calculation (e.g., converting relative time references), show your work
    5. Formulate a precise, concise answer based solely on the evidence in the memories
    6. Double-check that your answer directly addresses the question asked
    7. Ensure your final answer is specific and avoids vague time references

    Context:

    {context}

    Question: {question}
    Answer:
```

### 1.4 Letta LoCoMo Agent Prompt & Grader (`letta-ai/letta-leaderboard` @ `802a7942`)

- **Agent system prompt (`leaderboard/locomo/locomo_agent.txt`):** Instructs the agent to use `search_files` and finish with `answer_question(answer)`, convert relative time references, and return a precise concise answer.
- **Grader (`leaderboard/utils.py`, `GRADER_TEMPLATE`):** OpenAI SimpleQA-style 3-way template (`A: CORRECT`, `B: INCORRECT`, `C: NOT_ATTEMPTED`; scored `1.0 if A else 0.0`) run on `model="gpt-4.1"`, `temperature=0`. Skips Category 5 (`locomo_benchmark.py` lines 1632–1635).

---

## 2. LoCoMo — Generation 2 (`mem0ai/memory-benchmarks` @ `4b61c5d3`)

In 2026, Mem0 replaced `mem0/evaluation` with `mem0ai/memory-benchmarks`. Key differences from the 2025 paper:
- **Defaults:** `--answerer-model gpt-5`, `--judge-model gpt-5`, `--top-k 200`, `--top-k-cutoffs 10,20,50,200`, `--categories 1,2,3,4` (still skips Category 5).
- **Answer prompt (`benchmarks/locomo/prompts.py`, `ANSWER_GENERATION_PROMPT`):** Uses `ANSWERER_MEMORY_LIMIT = 200` chronological memories with human-readable dates, plus LoCoMo-specific date-range priors (`"These conversations took place around {reference_date}. All events occurred in 2022-2024. Never output 2025 or 2026."`), explicit anti-abstention (`"NEVER say 'not specified', 'not mentioned', 'no record'..."`), and final `"ANSWER:"` tag parsing. *(Warning: embedding `"All events occurred in 2022-2024"` in a Phase 0 or Phase 1 prompt leaks corpus-year priors; should NOT be copied blindly without owner awareness.)*
- **Judge prompt (`_JUDGE_TEMPLATE`):** `JUDGE_SYSTEM_PROMPT` + rules granting **partial credit** (at least 1 item from a list), **14-day date tolerance** and **50% duration tolerance**, and optional `--with-evidence` acceptance override.

---

## 3. LongMemEval-S — Verbatim Judge Prompts (`xiaowu0162/LongMemEval` @ `9e0b455f`)

From `src/evaluation/evaluate_qa.py` (default judge `gpt-4o-2024-08-06`, `temperature=0`):

1. **`single-session-user`, `single-session-assistant`, `multi-session`:**
   > `I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response is equivalent to the correct answer or contains all the intermediate steps to get the correct answer, you should also answer yes. If the response only contains a subset of the information required by the answer, answer no.`
2. **`temporal-reasoning`:** Same as above + `In addition, do not penalize off-by-one errors for the number of days. If the question asks for the number of days/weeks/months, etc., and the model makes off-by-one errors (e.g., predicting 19 days when the answer is 18), the model's response is still correct.`
3. **`knowledge-update`:** `I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response contains some previous information along with an updated answer, the response should be considered as correct as long as the updated answer is the required answer.`
4. **`single-session-preference`:** `I will give you a question, a rubric for desired personalized response, and a response from a model. Please answer yes if the response satisfies the desired response. Otherwise, answer no. The model does not need to reflect all the points in the rubric. The response is correct as long as it recalls and utilizes the user's personal information correctly.`
5. **Abstention (`_abs` in `question_id`):** `I will give you an unanswerable question, an explanation, and a response from a model. Please answer yes if the model correctly identifies the question as unanswerable. The model could say that the information is incomplete, or some other information is given but the asked information is not.`

---

## 4. Exact Published Scores (Verbatim from Primary Sources)

### 4.1 LoCoMo — Mem0 Paper (`arXiv:2504.19413v1`, Tables 1 & 2, Categories 1–4 = 1,540 QAs, `gpt-4o-mini`)

| Method | Single-Hop F1 / B1 / J | Multi-Hop F1 / B1 / J | Open-Domain F1 / B1 / J | Temporal F1 / B1 / J | Overall J (%) | Context Tokens | Search p50 / p95 (s) | Total p50 / p95 (s) |
|---|---|---|---|---|---|---|---|---|
| **LoCoMo** *(prior)* | 25.02 / 19.75 / – | 12.04 / 11.16 / – | 40.36 / 29.05 / – | 18.41 / 14.77 / – | – | – | – | – |
| **ReadAgent** *(prior)* | 9.15 / 6.48 / – | 5.31 / 5.12 / – | 9.67 / 7.66 / – | 12.60 / 8.87 / – | – | – | – | – |
| **MemoryBank** *(prior)* | 5.00 / 4.77 / – | 5.56 / 5.94 / – | 6.61 / 5.16 / – | 9.68 / 6.99 / – | – | – | – | – |
| **MemGPT** *(prior)* | 26.65 / 17.72 / – | 9.15 / 7.44 / – | 41.04 / 34.34 / – | 25.52 / 19.44 / – | – | – | – | – |
| **A-Mem** *(prior)* | 27.02 / 20.09 / – | 12.14 / 12.00 / – | 44.65 / 37.06 / – | 45.85 / 36.67 / – | – | – | – | – |
| **A-Mem\*** *(re-run)* | 20.76 / 14.90 / 39.79 ± 0.38 | 9.22 / 8.81 / 18.85 ± 0.31 | 33.34 / 27.58 / 54.05 ± 0.22 | 35.40 / 31.08 / 49.91 ± 0.31 | 48.38 ± 0.15 | 2,520 | 0.668 / 1.485 | 1.410 / 4.374 |
| **LangMem** | 35.51 / 26.86 / 62.23 ± 0.75 | 26.04 / 22.32 / 47.92 ± 0.47 | 40.91 / 33.63 / 71.12 ± 0.20 | 30.75 / 25.84 / 23.43 ± 0.39 | 58.10 ± 0.21 | 127 | 17.99 / 59.82 | 18.53 / 60.40 |
| **Zep** *(Mem0 eval)* | 35.74 / 23.30 / 61.70 ± 0.32 | 19.37 / 14.82 / 41.35 ± 0.48 | 49.56 / 38.92 / 76.60 ± 0.13 | 42.00 / 34.53 / 49.31 ± 0.50 | 65.99 ± 0.16 | 3,911 | 0.513 / 0.778 | 1.292 / 2.926 |
| **OpenAI** *(ChatGPT mem)* | 34.30 / 23.72 / 63.79 ± 0.46 | 20.09 / 15.42 / 42.92 ± 0.63 | 39.31 / 31.16 / 62.29 ± 0.12 | 14.04 / 11.25 / 21.71 ± 0.20 | 52.90 ± 0.14 | 4,437 | – / – | 0.466 / 0.889 |
| **RAG (best: k=2, 256 tok)** | – | – | – | – | 60.97 ± 0.20 | 512 | 0.255 / 0.699 | 0.802 / 1.907 |
| **Full-context** | – | – | – | – | 72.90 ± 0.19 | 26,031 | – / – | 9.870 / 17.117 |
| **Mem0** | 38.72 / 27.13 / 67.13 ± 0.65 | 28.64 / 21.58 / 51.15 ± 0.31 | 47.65 / 38.72 / 72.93 ± 0.11 | 48.93 / 40.51 / 55.51 ± 0.34 | 66.88 ± 0.15 | 1,764 | 0.148 / 0.200 | 0.708 / 1.440 |
| **Mem0^g** | 38.09 / 26.03 / 65.71 ± 0.45 | 24.32 / 18.82 / 47.19 ± 0.67 | 49.27 / 40.30 / 75.71 ± 0.21 | 51.55 / 40.28 / 58.13 ± 0.44 | 68.44 ± 0.17 | 3,616 | 0.476 / 0.657 | 1.091 / 2.590 |

### 4.2 LoCoMo — Author-Self-Reported Scores (Different Codebases / Models)

- **Zep (author replication, `getzep/zep-papers` @ `4b7f26cc`, `locomo_eval/README.md`, `gpt-4o-mini` answerer & judge, Cat 1–4 = 1,540 QAs):**
  - Overall J: **`75.14 ± 0.17%`** (p95 search latency `0.632s` per blog)
  - Single-Hop: **`79.79%`**, Multi-Hop: **`74.11%`**, Open-Domain: **`66.04%`**, Temporal: **`67.71%`**
- **Letta Filesystem (`letta.com/blog/benchmarking-ai-agent-memory`, Aug 12, 2025, `gpt-4o-mini` agent, `gpt-4.1` judge per `leaderboard/utils.py`, Cat 1–4 = 1,540 QAs):**
  - Overall accuracy: **`74.0%`** (per-category breakdown not printed in blog post or `leaderboard/locomo/`).
- **Mem0 Platform (`mem0ai/memory-benchmarks` @ `4b61c5d3`, `results/platform/locomo_results.json`, `gpt-5` answerer & judge, `top_k=200`, Cat 1–4 = 1,540 QAs):**
  - Overall accuracy: **`91.56%`** (`1410/1540`)
  - Multi-hop (Cat 1): **`93.26%`** (`263/282`), Temporal (Cat 2): **`92.83%`** (`298/321`), Open-domain (Cat 3): **`76.04%`** (`73/96`), Single-hop (Cat 4): **`92.27%`** (`776/841`).

### 4.3 LongMemEval-S — Zep Paper (`arXiv:2501.13956v1`, Tables 2 & 3, 500 QAs, `gpt-4o` judge)

- **Overall (`Table 2`):**
  - Full-context (`gpt-4o-mini`): **`55.4%`** (mean latency `31.3 s`, IQR `8.76 s`, ~`115k` context tokens)
  - Zep (`gpt-4o-mini`): **`63.8%`** (mean latency `3.20 s`, IQR `1.31 s`, `1.6k` context tokens)
  - Full-context (`gpt-4o`): **`60.2%`** (mean latency `28.9 s`, IQR `6.01 s`, ~`115k` context tokens)
  - Zep (`gpt-4o`): **`71.2%`** (mean latency `2.58 s`, IQR `0.684 s`, `1.6k` context tokens)
- **By Question Type (`Table 3`, Full-context vs Zep):**
  - `gpt-4o-mini`: `single-session-preference` `30.0%` vs **`53.3%`**; `single-session-assistant` **`81.8%`** vs `75.0%`; `temporal-reasoning` `36.5%` vs **`54.1%`**; `multi-session` `40.6%` vs **`47.4%`**; `knowledge-update` **`76.9%`** vs `74.4%`; `single-session-user` `81.4%` vs **`92.9%`**.
  - `gpt-4o`: `single-session-preference` `20.0%` vs **`56.7%`**; `single-session-assistant` **`94.6%`** vs `80.4%`; `temporal-reasoning` `45.1%` vs **`62.4%`**; `multi-session` `44.3%` vs **`57.9%`**; `knowledge-update` `78.2%` vs **`83.3%`**; `single-session-user` `81.4%` vs **`92.9%`**.
- **Note on Mem0 LongMemEval-S (`mem0ai/memory-benchmarks` @ `4b61c5d3`, `gpt-5` answerer & judge, `top_k=200`):**
  - Platform (`results/platform/longmemeval_results.json`): **`93.4%`** (`467/500`) — `knowledge-update`: `96.15%`, `multi-session`: `86.47%`, `single-session-assistant`: `100.0%`, `single-session-preference`: `96.67%`, `single-session-user`: `97.14%`, `temporal-reasoning`: `93.23%` (calculated `124/133`).
  - OSS (`results/oss/longmemeval_gpt5.json`): **`91.0%`** (`455/500`) — `knowledge-update`: `91.03%`, `multi-session`: `83.46%`, `single-session-assistant`: `92.86%`, `single-session-preference`: `93.33%`, `single-session-user`: `95.71%`, `temporal-reasoning`: `94.74%`.
