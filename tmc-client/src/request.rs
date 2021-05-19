//! Contains types used to make requests to the TMC server.

/// Used to respond to feedback questions. See TmcClient::send_feedback.
pub struct FeedbackAnswer {
    pub question_id: u32,
    pub answer: String,
}
