use crate::{
    models::LlmStatusResponse,
    AppResult, AppState,
};
use axum::{extract::State, Json};

#[utoipa::path(
    get,
    path = "/api/llm/status",
    tag = "LLM",
    responses(
        (status = 200, description = "LLM connectivity from the backend container", body = LlmStatusResponse)
    )
)]
pub async fn get_llm_status(State(state): State<AppState>) -> AppResult<Json<LlmStatusResponse>> {
    let result = state.gemma.list_model_ids().await;

    let (reachable, model_ids, error_msg) = match result {
        Ok(model_ids) => (true, model_ids, None),
        Err(error) => (false, Vec::new(), Some(error.to_string())),
    };

    Ok(Json(LlmStatusResponse {
        base_url: state.gemma.base_url().to_string(),
        configured_model: state.gemma.model().to_string(),
        reachable,
        model_ids,
        error_msg,
    }))
}
