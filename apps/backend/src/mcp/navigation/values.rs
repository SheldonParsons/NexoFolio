//! Full typed observation values keep their existing archive identity.
use super::*;
impl Navigation {
    pub(super) async fn values(&self, request: &Query) -> Result<Value, ReadError> {
        let q = nexofolio_contracts::field_values::FieldValueQuery {
            project: request.project_id,
            endpoint: request.endpoint_id.ok_or("ENDPOINT_REQUIRED")?,
            archive: request.archive_id.ok_or("ARCHIVE_REQUIRED")?,
            field: request.field_key.clone().ok_or("FIELD_REQUIRED")?,
            offset: request.offset,
            limit: request.limit,
            environment: request.environment_id,
        };
        if let Some(value_id) = &request.value_id {
            let value = self
                .knowledge
                .observed_value(q, value_id)
                .await
                .map_err(|_| "VALUE_UNAVAILABLE")?;
            let encoded = serde_json::to_string(&value).map_err(|_| "VALUE_UNAVAILABLE")?;
            let total = encoded.chars().count();
            if request.content_offset > total {
                return Err(ReadError::bounds(
                    "content_offset",
                    request.content_offset,
                    0,
                    total,
                    0,
                    "characters",
                ));
            }
            let data: String = encoded
                .chars()
                .skip(request.content_offset)
                .take(request.max_chars)
                .collect();
            let next = request.content_offset + data.chars().count();
            Ok(
                json!({"archive_id":request.archive_id,"value_id":value_id,"encoding":"json_text_chunks","data":data,"next_offset":next,"total_chars":total,"has_more":next<total}),
            )
        } else {
            self.knowledge
                .observed_values(q)
                .await
                .map_err(|_| ReadError::from("VALUE_UNAVAILABLE"))
                .and_then(|v| {
                    serde_json::to_value(v).map_err(|_| ReadError::from("VALUE_UNAVAILABLE"))
                })
        }
    }
}
