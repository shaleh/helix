use std::path::{Path, PathBuf};

/// States for a file having been changed.
pub enum FileChange {
    /// Not tracked by the VCS.
    Untracked { path: PathBuf },
    /// File has been modified.
    Modified { path: PathBuf },
    /// File modification is in conflict with a different update.
    Conflict { path: PathBuf },
    /// File has been deleted.
    Deleted { path: PathBuf },
    /// File has been renamed.
    Renamed {
        from_path: PathBuf,
        to_path: PathBuf,
    },
}

impl FileChange {
    pub fn path(&self) -> &Path {
        match self {
            Self::Untracked { path } => path,
            Self::Modified { path } => path,
            Self::Conflict { path } => path,
            Self::Deleted { path } => path,
            Self::Renamed { to_path, .. } => to_path,
        }
    }

    /// Returns a sort key that orders by change type (conflicts first) then alphabetically by path.
    pub fn sort_key(&self) -> (u8, &Path) {
        let priority = match self {
            Self::Conflict { .. } => 0,
            Self::Modified { .. } => 1,
            Self::Renamed { .. } => 2,
            Self::Deleted { .. } => 3,
            Self::Untracked { .. } => 4,
        };
        (priority, self.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_key_orders_conflicts_first() {
        let mut changes = vec![
            FileChange::Untracked {
                path: "new.txt".into(),
            },
            FileChange::Modified {
                path: "changed.txt".into(),
            },
            FileChange::Conflict {
                path: "merge.txt".into(),
            },
            FileChange::Deleted {
                path: "gone.txt".into(),
            },
            FileChange::Renamed {
                from_path: "old.txt".into(),
                to_path: "moved.txt".into(),
            },
        ];

        changes.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

        assert!(matches!(changes[0], FileChange::Conflict { .. }));
        assert!(matches!(changes[1], FileChange::Modified { .. }));
        assert!(matches!(changes[2], FileChange::Renamed { .. }));
        assert!(matches!(changes[3], FileChange::Deleted { .. }));
        assert!(matches!(changes[4], FileChange::Untracked { .. }));
    }

    #[test]
    fn sort_key_alphabetical_within_same_type() {
        let mut changes = vec![
            FileChange::Conflict {
                path: "z_file.txt".into(),
            },
            FileChange::Conflict {
                path: "a_file.txt".into(),
            },
            FileChange::Modified {
                path: "b_mod.txt".into(),
            },
            FileChange::Modified {
                path: "a_mod.txt".into(),
            },
        ];

        changes.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

        assert_eq!(changes[0].path(), Path::new("a_file.txt"));
        assert_eq!(changes[1].path(), Path::new("z_file.txt"));
        assert_eq!(changes[2].path(), Path::new("a_mod.txt"));
        assert_eq!(changes[3].path(), Path::new("b_mod.txt"));
    }
}
