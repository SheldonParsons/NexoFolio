//! Composition of capture admission and authenticated evidence storage ports.

pub trait CaptureGateway:
    nexofolio_evidence::CaptureEvidenceRepository + nexofolio_intake::CaptureAdmissionStore
{
}
impl<T> CaptureGateway for T where
    T: nexofolio_evidence::CaptureEvidenceRepository + nexofolio_intake::CaptureAdmissionStore
{
}
