use std::fmt::Display;

#[derive(Debug)]
pub struct Recommendation {
    pub site: String,
    pub direction: RecommendationDirection,
    pub action: RecommendationAction,
}


#[derive(Debug, Copy, Clone)]
pub enum RecommendationDirection {
    Download,
    Upload,
}

impl Display for RecommendationDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecommendationDirection::Download => write!(f, "Download"),
            RecommendationDirection::Upload => write!(f, "Upload"),
        }
    }
}

#[derive(Debug)]
pub enum RecommendationAction {
    IncreaseFast,
    Increase,
    Decrease,
    DecreaseFast,
}