//! Verbatim benchmark question-format instructions, judge prompts and scoring helpers used by
//! the memory suite (`Design 2`, `Design 3`, `R-55`).

/// Answer-format instruction appended to every LoCoMo question percept (`answer_format_v1`).
pub const LOCOMO_ANSWER_FORMAT_V1: &str = "Answer briefly, in at most a few words. Convert relative time references to specific dates, months or years. If the conversation does not contain the answer, reply exactly: No information available.";

/// Answer-format instruction appended to every LongMemEval-S question percept (`answer_format_v1`).
pub const LONGMEMEVAL_ANSWER_FORMAT_V1: &str = "Briefly answer the question. If you don't know how to answer the question, abstain from answering and say so.";

/// Recorded prompt version for the answer-format instruction.
pub const ANSWER_FORMAT_PROMPT_VERSION: &str = crate::memory::proxy::ANSWER_FORMAT_V1;

/// LoCoMo `ACCURACY_PROMPT` from `mem0ai/mem0` `evaluation/metrics/llm_judge.py`, verbatim.
pub const LOCOMO_ACCURACY_PROMPT: &str = r#"Your task is to label an answer to a question as ’CORRECT’ or ’WRONG’. You will be given the following data:
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
"#;

/// LongMemEval judge prompt for `single-session-user`, `single-session-assistant` and `multi-session`.
pub const LONGMEMEVAL_PROMPT_STANDARD: &str = "I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response is equivalent to the correct answer or contains all the intermediate steps to get the correct answer, you should also answer yes. If the response only contains a subset of the information required by the answer, answer no. \n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.";

/// LongMemEval judge prompt for `temporal-reasoning` (off-by-one tolerance).
pub const LONGMEMEVAL_PROMPT_TEMPORAL: &str = "I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response is equivalent to the correct answer or contains all the intermediate steps to get the correct answer, you should also answer yes. If the response only contains a subset of the information required by the answer, answer no. In addition, do not penalize off-by-one errors for the number of days. If the question asks for the number of days/weeks/months, etc., and the model makes off-by-one errors (e.g., predicting 19 days when the answer is 18), the model's response is still correct. \n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.";

/// LongMemEval judge prompt for `knowledge-update`.
pub const LONGMEMEVAL_PROMPT_KNOWLEDGE_UPDATE: &str = "I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response contains some previous information along with an updated answer, the response should be considered as correct as long as the updated answer is the required answer.\n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.";

/// LongMemEval judge prompt for `single-session-preference` (rubric).
pub const LONGMEMEVAL_PROMPT_PREFERENCE: &str = "I will give you a question, a rubric for desired personalized response, and a response from a model. Please answer yes if the response satisfies the desired response. Otherwise, answer no. The model does not need to reflect all the points in the rubric. The response is correct as long as it recalls and utilizes the user's personal information correctly.\n\nQuestion: {}\n\nRubric: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only.";

/// LongMemEval judge prompt for `_abs` abstention items.
pub const LONGMEMEVAL_PROMPT_ABSTENTION: &str = "I will give you an unanswerable question, an explanation, and a response from a model. Please answer yes if the model correctly identifies the question as unanswerable. The model could say that the information is incomplete, or some other information is given but the asked information is not.\n\nQuestion: {}\n\nExplanation: {}\n\nModel Response: {}\n\nDoes the model correctly identify the question as unanswerable? Answer yes or no only.";

/// Prompt version for LongMemEval-S judging (`R-55` templates, official `evaluate_qa.py`).
pub const LONGMEMEVAL_PROMPT_VERSION: &str = "longmemeval_official_2024_v1";

/// Prompt version for LoCoMo judging.
pub const LOCOMO_PROMPT_VERSION: &str = "locomo_accuracy_v1";

/// Builds the LoCoMo judge user prompt from the verbatim template.
#[must_use]
pub fn locomo_judge_prompt(question: &str, gold: &str, answer: &str) -> String {
    LOCOMO_ACCURACY_PROMPT
        .replace("{question}", question)
        .replace("{gold_answer}", gold)
        .replace("{generated_answer}", answer)
}

/// Builds the LongMemEval-S judge user prompt for one question type, exactly as the official
/// `get_anscheck_prompt` does.
#[must_use]
pub fn longmemeval_judge_prompt(
    question_type: &str,
    question: &str,
    answer: &str,
    response: &str,
    is_abs: bool,
) -> String {
    let template = if is_abs {
        LONGMEMEVAL_PROMPT_ABSTENTION
    } else {
        match question_type {
            "temporal-reasoning" => LONGMEMEVAL_PROMPT_TEMPORAL,
            "knowledge-update" => LONGMEMEVAL_PROMPT_KNOWLEDGE_UPDATE,
            "single-session-preference" => LONGMEMEVAL_PROMPT_PREFERENCE,
            _ => LONGMEMEVAL_PROMPT_STANDARD,
        }
    };
    template
        .replacen("{}", question, 1)
        .replacen("{}", answer, 1)
        .replacen("{}", response, 1)
}

/// Parses a LoCoMo judge response: `{"label": "CORRECT"|"WRONG"}` (tolerates surrounding text).
#[must_use]
pub fn parse_locomo_verdict(raw: &str) -> Option<bool> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw.trim())
        && let Some(label) = value.get("label").and_then(serde_json::Value::as_str)
    {
        return label_verdict(label);
    }
    // Fall back to the last JSON object in the text, then to bare labels.
    if let Some(start) = raw.rfind('{')
        && let Some(end) = raw[start..].find('}')
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw[start..=start + end])
        && let Some(label) = value.get("label").and_then(serde_json::Value::as_str)
    {
        return label_verdict(label);
    }
    let upper = raw.to_ascii_uppercase();
    if upper.contains("CORRECT") && !upper.contains("WRONG") {
        return Some(true);
    }
    if upper.contains("WRONG") && !upper.contains("CORRECT") {
        return Some(false);
    }
    None
}

fn label_verdict(label: &str) -> Option<bool> {
    match label.trim().to_ascii_uppercase().as_str() {
        "CORRECT" => Some(true),
        "WRONG" => Some(false),
        _ => None,
    }
}

/// Parses a LongMemEval judge response exactly as the official script (`'yes' in response.lower()`).
#[must_use]
pub fn parse_longmemeval_verdict(raw: &str) -> Option<bool> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&lower)
        && let Some(label) = value.get("label").and_then(serde_json::Value::as_str)
    {
        let label = label.trim().to_ascii_lowercase();
        if label.contains("yes") {
            return Some(true);
        }
        if label.contains("no") {
            return Some(false);
        }
    }
    if lower.contains("yes") && !lower.contains("no") {
        return Some(true);
    }
    if lower.contains("no") && !lower.contains("yes") {
        return Some(false);
    }
    None
}
