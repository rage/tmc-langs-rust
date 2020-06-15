//! Contains types used to make requests to the TMC server.

pub struct FeedbackAnswer {
    pub question_id: usize,
    pub answer: String,
}
