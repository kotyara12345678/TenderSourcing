//! Tender state machine (pure functions, fully unit tested).
//!
//! The status values and legal transitions mirror `docs/ARCHITECTURE.md` and
//! the `tenders.status` CHECK constraint. The orchestrator is the only
//! component allowed to write `tenders.status`; every write goes through
//! [`Status::transition_to`] so an illegal move is a programming error, not
//! a silent data corruption.

/// All tender statuses in pipeline order.
pub const ALL_STATUSES: [&str; 16] = [
    "CREATED",
    "UPLOADING",
    "DOCUMENT_PROCESSING",
    "REQUIREMENT_EXTRACTION",
    "REQUIREMENTS_REVIEW",
    "SEARCH_QUEUED",
    "SEARCHING",
    "FETCHING",
    "PRODUCT_EXTRACTION",
    "MATCHING",
    "RANKING",
    "REPORT_GENERATION",
    "COMPLETED",
    "COMPLETED_WITH_ERRORS",
    "FAILED",
    "CANCELLED",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Created,
    Uploading,
    DocumentProcessing,
    RequirementExtraction,
    RequirementsReview,
    SearchQueued,
    Searching,
    Fetching,
    ProductExtraction,
    Matching,
    Ranking,
    ReportGeneration,
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Created => "CREATED",
            Status::Uploading => "UPLOADING",
            Status::DocumentProcessing => "DOCUMENT_PROCESSING",
            Status::RequirementExtraction => "REQUIREMENT_EXTRACTION",
            Status::RequirementsReview => "REQUIREMENTS_REVIEW",
            Status::SearchQueued => "SEARCH_QUEUED",
            Status::Searching => "SEARCHING",
            Status::Fetching => "FETCHING",
            Status::ProductExtraction => "PRODUCT_EXTRACTION",
            Status::Matching => "MATCHING",
            Status::Ranking => "RANKING",
            Status::ReportGeneration => "REPORT_GENERATION",
            Status::Completed => "COMPLETED",
            Status::CompletedWithErrors => "COMPLETED_WITH_ERRORS",
            Status::Failed => "FAILED",
            Status::Cancelled => "CANCELLED",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        ALL_STATUSES
            .iter()
            .position(|candidate| *candidate == value)
            .map(|index| match index {
                0 => Status::Created,
                1 => Status::Uploading,
                2 => Status::DocumentProcessing,
                3 => Status::RequirementExtraction,
                4 => Status::RequirementsReview,
                5 => Status::SearchQueued,
                6 => Status::Searching,
                7 => Status::Fetching,
                8 => Status::ProductExtraction,
                9 => Status::Matching,
                10 => Status::Ranking,
                11 => Status::ReportGeneration,
                12 => Status::Completed,
                13 => Status::CompletedWithErrors,
                14 => Status::Failed,
                _ => Status::Cancelled,
            })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Status::Completed | Status::CompletedWithErrors | Status::Failed | Status::Cancelled
        )
    }

    /// The next stage in the happy-path pipeline (`None` at the end).
    pub fn successor(self) -> Option<Status> {
        match self {
            Status::Created => Some(Status::Uploading),
            Status::Uploading => Some(Status::DocumentProcessing),
            Status::DocumentProcessing => Some(Status::RequirementExtraction),
            Status::RequirementExtraction => Some(Status::RequirementsReview),
            Status::RequirementsReview => Some(Status::SearchQueued),
            Status::SearchQueued => Some(Status::Searching),
            Status::Searching => Some(Status::Fetching),
            Status::Fetching => Some(Status::ProductExtraction),
            Status::ProductExtraction => Some(Status::Matching),
            Status::Matching => Some(Status::Ranking),
            Status::Ranking => Some(Status::ReportGeneration),
            Status::ReportGeneration => Some(Status::Completed),
            _ => None,
        }
    }

    /// Is `from → to` a legal transition?
    ///
    /// Terminal states are immutable; every non-terminal state may fail or be
    /// cancelled; re-applying the current state is an idempotent no-op.
    pub fn can_transition(from: Status, to: Status) -> bool {
        if from == to {
            return true;
        }
        if from.is_terminal() {
            return false;
        }
        if matches!(to, Status::Failed | Status::Cancelled) {
            return true;
        }
        match from {
            Status::Created => matches!(to, Status::Uploading | Status::DocumentProcessing),
            Status::Uploading => matches!(
                to,
                Status::DocumentProcessing | Status::RequirementExtraction
            ),
            Status::DocumentProcessing => matches!(
                to,
                Status::RequirementExtraction | Status::RequirementsReview
            ),
            Status::RequirementExtraction => {
                matches!(to, Status::RequirementsReview | Status::SearchQueued)
            }
            Status::RequirementsReview => matches!(to, Status::SearchQueued),
            Status::SearchQueued => matches!(to, Status::Searching),
            Status::Searching => matches!(to, Status::Fetching | Status::SearchQueued),
            Status::Fetching => matches!(to, Status::ProductExtraction),
            Status::ProductExtraction => matches!(to, Status::Matching),
            Status::Matching => matches!(to, Status::Ranking),
            Status::Ranking => matches!(to, Status::ReportGeneration),
            Status::ReportGeneration => {
                matches!(to, Status::Completed | Status::CompletedWithErrors)
            }
            Status::Completed
            | Status::CompletedWithErrors
            | Status::Failed
            | Status::Cancelled => false,
        }
    }

    /// Validate and return the resulting status.
    pub fn transition_to(self, to: Status) -> Result<Status, TransitionError> {
        if Status::can_transition(self, to) {
            Ok(to)
        } else {
            Err(TransitionError {
                from: self.as_str(),
                to: to.as_str(),
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionError {
    pub from: &'static str,
    pub to: &'static str,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "illegal tender transition {} -> {}", self.from, self.to)
    }
}

impl std::error::Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_round_trips_through_parse() {
        for value in ALL_STATUSES {
            let status = Status::parse(value).expect("known status");
            assert_eq!(status.as_str(), value);
        }
        assert_eq!(Status::parse("NOPE"), None);
    }

    #[test]
    fn successor_chain_reaches_the_end() {
        let mut status = Status::Created;
        let mut steps = 0;
        while let Some(next) = status.successor() {
            assert!(
                Status::can_transition(status, next),
                "{} -> {}",
                status.as_str(),
                next.as_str()
            );
            status = next;
            steps += 1;
            assert!(steps < 20, "successor chain must terminate");
        }
        assert_eq!(status, Status::Completed);
    }

    #[test]
    fn happy_path_is_legal() {
        assert!(Status::can_transition(
            Status::SearchQueued,
            Status::Searching
        ));
        assert!(Status::can_transition(Status::Searching, Status::Fetching));
        assert!(Status::can_transition(
            Status::ReportGeneration,
            Status::Completed
        ));
        assert!(Status::can_transition(
            Status::ReportGeneration,
            Status::CompletedWithErrors
        ));
    }

    #[test]
    fn illegal_backwards_transition_is_rejected() {
        let err = Status::Matching
            .transition_to(Status::Searching)
            .unwrap_err();
        assert_eq!(err.from, "MATCHING");
        assert_eq!(err.to, "SEARCHING");
        assert!(Status::Ranking.transition_to(Status::Fetching).is_err());
    }

    #[test]
    fn terminal_states_are_immutable() {
        for terminal in [
            Status::Completed,
            Status::CompletedWithErrors,
            Status::Failed,
            Status::Cancelled,
        ] {
            for target in ALL_STATUSES {
                let target = Status::parse(target).unwrap();
                if target == terminal {
                    assert!(Status::can_transition(terminal, target));
                } else {
                    assert!(
                        !Status::can_transition(terminal, target),
                        "{} must not move to {}",
                        terminal.as_str(),
                        target.as_str()
                    );
                }
            }
        }
    }

    #[test]
    fn any_active_state_can_fail_or_cancel() {
        for value in ALL_STATUSES {
            let status = Status::parse(value).unwrap();
            if status.is_terminal() {
                continue;
            }
            assert!(Status::can_transition(status, Status::Failed));
            assert!(Status::can_transition(status, Status::Cancelled));
        }
    }

    #[test]
    fn same_state_is_idempotent() {
        for value in ALL_STATUSES {
            let status = Status::parse(value).unwrap();
            assert!(Status::can_transition(status, status));
        }
    }
}
