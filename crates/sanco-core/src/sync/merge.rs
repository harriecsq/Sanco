//! Merge rule for replicated rows.
//!
//! Versions are totally ordered by `(deleted, hlc, site)`: deletes win, then
//! the newest HLC, then the site id as a tiebreak. Every peer keeps the
//! greatest version it has seen, so all peers converge whatever order rows
//! arrive in.
//!
//! Each row also carries a version vector (`_vv`: site → last HLC that site
//! wrote to the row). If neither version's vector covers the other, the two
//! edits were made without seeing each other, and the losing one is recorded
//! as a conflict instead of vanishing silently.

use std::collections::BTreeMap;

use super::hlc::Hlc;

pub type VersionVector = BTreeMap<String, Hlc>;

/// True if `a` has seen everything `b` has.
pub fn dominates(a: &VersionVector, b: &VersionVector) -> bool {
    b.iter()
        .all(|(site, h)| a.get(site).is_some_and(|x| x >= h))
}

#[derive(Debug, Clone)]
pub struct Version<'a> {
    pub hlc: Hlc,
    pub site: &'a str,
    pub deleted: bool,
    pub vv: &'a VersionVector,
}

impl Version<'_> {
    fn order_key(&self) -> (bool, Hlc, &str) {
        (self.deleted, self.hlc, self.site)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loser {
    Local,
    Incoming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// Replace the local row with the incoming one.
    pub take_incoming: bool,
    /// The losing version was a concurrent edit that should be recorded.
    pub conflict: Option<Loser>,
}

pub fn decide(local: &Version, incoming: &Version) -> Decision {
    let ord = incoming.order_key().cmp(&local.order_key());
    if ord.is_eq() {
        // Same version, e.g. an echo of our own write.
        return Decision {
            take_incoming: false,
            conflict: None,
        };
    }
    let take_incoming = ord.is_gt();
    let concurrent = !dominates(local.vv, incoming.vv) && !dominates(incoming.vv, local.vv);
    let (loser, loser_side) = if take_incoming {
        (local, Loser::Local)
    } else {
        (incoming, Loser::Incoming)
    };
    // A losing delete needs no record: the row ends up deleted either way.
    let conflict = (concurrent && !loser.deleted).then_some(loser_side);
    Decision {
        take_incoming,
        conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vv(entries: &[(&str, i64)]) -> VersionVector {
        entries
            .iter()
            .map(|(s, h)| (s.to_string(), Hlc(*h)))
            .collect()
    }

    fn v<'a>(hlc: i64, site: &'a str, deleted: bool, vv: &'a VersionVector) -> Version<'a> {
        Version {
            hlc: Hlc(hlc),
            site,
            deleted,
            vv,
        }
    }

    #[test]
    fn successor_replaces_without_conflict() {
        let old = vv(&[("a", 1)]);
        let new = vv(&[("a", 1), ("b", 2)]);
        let d = decide(&v(1, "a", false, &old), &v(2, "b", false, &new));
        assert_eq!(
            d,
            Decision {
                take_incoming: true,
                conflict: None
            }
        );
        // Stale version arriving late is ignored quietly.
        let d = decide(&v(2, "b", false, &new), &v(1, "a", false, &old));
        assert_eq!(
            d,
            Decision {
                take_incoming: false,
                conflict: None
            }
        );
    }

    #[test]
    fn concurrent_edits_record_the_loser() {
        let a = vv(&[("a", 5)]);
        let b = vv(&[("a", 1), ("b", 6)]);
        let d = decide(&v(5, "a", false, &a), &v(6, "b", false, &b));
        assert_eq!(d.conflict, Some(Loser::Local));
        assert!(d.take_incoming);
        let d = decide(&v(6, "b", false, &b), &v(5, "a", false, &a));
        assert_eq!(d.conflict, Some(Loser::Incoming));
        assert!(!d.take_incoming);
    }

    #[test]
    fn delete_beats_newer_concurrent_edit() {
        let del = vv(&[("a", 5)]);
        let edit = vv(&[("b", 9)]);
        let d = decide(&v(9, "b", false, &edit), &v(5, "a", true, &del));
        assert!(d.take_incoming);
        assert_eq!(d.conflict, Some(Loser::Local));
    }

    #[test]
    fn site_breaks_hlc_ties_and_echo_is_ignored() {
        let a = vv(&[("a", 5)]);
        let b = vv(&[("b", 5)]);
        assert!(decide(&v(5, "a", false, &a), &v(5, "b", false, &b)).take_incoming);
        assert!(!decide(&v(5, "b", false, &b), &v(5, "a", false, &a)).take_incoming);
        let d = decide(&v(5, "a", false, &a), &v(5, "a", false, &a));
        assert!(!d.take_incoming && d.conflict.is_none());
    }
}
