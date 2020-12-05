//! Contains types used to make requests to the TMC server.

/// Used to respond to feedback questions. See TmcClient::send_feedback.
pub struct FeedbackAnswer {
    pub question_id: usize,
    pub answer: String,
}
