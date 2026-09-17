fn main() {
    use nexofolio_contracts::{
        ObservationAssessment, ObservationAssessmentPage, ObservationFieldRef,
    };
    println!(
        "{}",
        serde_json::json!({
            "observed-field-ref.schema.json":schemars::schema_for!(ObservationFieldRef),
        "assessment.schema.json":schemars::schema_for!(ObservationAssessment),
            "assessment-page.schema.json":schemars::schema_for!(ObservationAssessmentPage)
        })
    );
}
