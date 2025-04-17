use std::fmt::Display;

#[derive(Debug)]
pub struct Recommendation {
    pub site: String,
    pub direction: RecommendationDirection,
    pub action: RecommendationAction,
}

impl Recommendation {
    pub fn new(
        site: &str,
        direction: RecommendationDirection,
        action: RecommendationAction,
    ) -> Self {
        Self {
            site: site.to_string(),
            direction,
            action,
        }
    }
}


#[derive(Debug)]
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