use std::fmt::Display;

use allocative::Allocative;

#[derive(Debug, Allocative)]
pub struct Recommendation {
    pub site: String,
    pub direction: RecommendationDirection,
    pub action: RecommendationAction,
}


#[derive(Debug, Copy, Clone, Allocative)]
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

#[derive(Debug, Allocative)]
pub enum RecommendationAction {
    IncreaseFast,
    Increase,
    Decrease,
    DecreaseFast,
}