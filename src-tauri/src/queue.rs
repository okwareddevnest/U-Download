use crate::jobs::{JobId, JobProgress, JobRegistry, JobStatus};

/// Returns the job ids that may start right now, respecting the concurrency
/// limit and the slots already occupied by in-flight work.
pub fn next_dispatchable(reg: &JobRegistry, concurrency: usize) -> Vec<JobId> {
    let limit = concurrency.max(1);
    let free = limit.saturating_sub(reg.active_count());
    reg.queued_ids().into_iter().take(free).collect()
}

/// Atomically decides what may start and claims it.
///
/// `next_dispatchable` alone is not safe to act on: it reports jobs that are
/// still `Queued`, and the runner calls the scheduler from every finishing job
/// thread. Two callers overlapping between the decision and the status write
/// would each hand the same job to a thread and download it twice, at once,
/// to one output path. Doing both under one borrow of the registry — which the
/// caller holds under one lock — closes that window. `Probing` counts towards
/// `active_count`, so the slot is spoken for the moment the borrow ends.
pub fn claim_next(reg: &mut JobRegistry, concurrency: usize) -> Vec<JobId> {
    let claimed = next_dispatchable(reg, concurrency);
    for id in &claimed {
        reg.set_status(id, JobStatus::Probing);
    }
    claimed
}

/// Moves a claimed job into `Downloading`, reporting whether it was still the
/// caller's to move.
///
/// Between the claim and the process actually being spawned, a cancel or a
/// pause can arrive. It finds no child to kill — there is none yet — and
/// simply writes its own terminal status. Promoting unconditionally would
/// overwrite that, and the run would carry on to report `Done` with the user's
/// cancel silently discarded. A `false` return means: kill what you spawned
/// and leave the status alone.
pub fn promote_to_downloading(reg: &mut JobRegistry, id: &JobId) -> bool {
    if reg.get(id).map(|job| job.status) != Some(JobStatus::Probing) {
        return false;
    }
    reg.set_status(id, JobStatus::Downloading);
    true
}

/// Whether the job is still owned by the run that promoted it.
///
/// `Downloading` is written only by `promote_to_downloading`, so any other
/// status means a cancel or pause took the job over and published its own
/// terminal state. The finishing run must then report nothing at all — a
/// killed process exits non-zero, and reporting that would turn a pause into
/// a failure in the UI.
pub fn still_running(reg: &JobRegistry, id: &JobId) -> bool {
    reg.get(id).map(|job| job.status) == Some(JobStatus::Downloading)
}

/// Pauses a job. There is no process-level suspend: a job that is already
/// downloading is killed outright and its progress discarded, so resuming
/// restarts it from zero. The UI must present it that way.
pub fn pause(reg: &mut JobRegistry, id: &JobId) {
    let status = match reg.get(id) {
        Some(job) => job.status,
        None => return,
    };

    // A finished job's outcome (Done/Failed/Cancelled) must never be
    // overwritten by a later pause call — the transition below is only for
    // jobs still queued or actively running.
    if status.is_terminal() {
        return;
    }

    if status.is_active() {
        reg.cancel(id);
        reg.update_progress(id, JobProgress::default());
    }

    reg.set_status(id, JobStatus::Paused);
}

pub fn resume(reg: &mut JobRegistry, id: &JobId) {
    if reg.get(id).map(|j| j.status) == Some(JobStatus::Paused) {
        reg.set_status(id, JobStatus::Queued);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{Job, JobRegistry, JobStatus};
    use crate::ytdlp::{FormatChoice, MediaKind};

    fn reg_with(n: usize) -> (JobRegistry, Vec<String>) {
        let mut reg = JobRegistry::new();
        let ids = (0..n)
            .map(|_| {
                reg.insert(Job::new(
                    "https://example.com/v".to_string(),
                    FormatChoice::Quick { kind: MediaKind::Mp4, height: None },
                    None,
                    "/out".to_string(),
                ))
            })
            .collect();
        (reg, ids)
    }

    #[test]
    fn dispatches_up_to_the_concurrency_limit() {
        let (reg, _) = reg_with(5);
        assert_eq!(next_dispatchable(&reg, 2).len(), 2);
    }

    #[test]
    fn dispatches_in_fifo_order() {
        let (reg, ids) = reg_with(3);
        assert_eq!(next_dispatchable(&reg, 2), vec![ids[0].clone(), ids[1].clone()]);
    }

    #[test]
    fn accounts_for_already_active_jobs() {
        let (mut reg, ids) = reg_with(4);
        reg.set_status(&ids[0], JobStatus::Downloading);
        // One slot of two is taken, so only one more may start.
        assert_eq!(next_dispatchable(&reg, 2).len(), 1);
    }

    #[test]
    fn dispatches_nothing_when_saturated() {
        let (mut reg, ids) = reg_with(4);
        reg.set_status(&ids[0], JobStatus::Downloading);
        reg.set_status(&ids[1], JobStatus::Processing);
        assert!(next_dispatchable(&reg, 2).is_empty());
    }

    #[test]
    fn skips_paused_jobs() {
        let (mut reg, ids) = reg_with(3);
        reg.set_status(&ids[0], JobStatus::Paused);
        assert_eq!(next_dispatchable(&reg, 1), vec![ids[1].clone()]);
    }

    #[test]
    fn pausing_a_queued_job_removes_it_from_dispatch() {
        let (mut reg, ids) = reg_with(2);
        pause(&mut reg, &ids[0]);
        assert_eq!(reg.get(&ids[0]).unwrap().status, JobStatus::Paused);
        assert_eq!(next_dispatchable(&reg, 4), vec![ids[1].clone()]);
    }

    // Spec section 4.4: pause is non-suspending — an in-flight job is killed.
    #[test]
    fn pausing_a_downloading_job_cancels_it_and_resets_progress() {
        let (mut reg, ids) = reg_with(1);
        reg.set_status(&ids[0], JobStatus::Downloading);
        reg.update_progress(&ids[0], crate::jobs::JobProgress { percentage: 55.0, ..Default::default() });

        pause(&mut reg, &ids[0]);

        let job = reg.get(&ids[0]).unwrap();
        assert_eq!(job.status, JobStatus::Paused);
        assert_eq!(job.progress.percentage, 0.0, "progress is not preserved across pause");
    }

    // The bug this exists to prevent: `pump` runs from every finishing job
    // thread, so two claims can overlap. If a claim did not mark what it
    // returned, both would get the same id and yt-dlp would run twice against
    // one output path.
    #[test]
    fn two_consecutive_claims_never_return_the_same_job() {
        let (mut reg, _) = reg_with(5);
        let first = claim_next(&mut reg, 4);
        let second = claim_next(&mut reg, 4);

        assert_eq!(first.len(), 4);
        assert!(second.is_empty(), "the first claim already took every slot");
        for id in &first {
            assert!(!second.contains(id));
        }
    }

    #[test]
    fn a_claim_consumes_a_concurrency_slot() {
        let (mut reg, _) = reg_with(4);
        assert_eq!(reg.active_count(), 0);

        let claimed = claim_next(&mut reg, 2);

        assert_eq!(claimed.len(), 2);
        assert_eq!(reg.active_count(), 2, "Probing has to count as in-flight");
        assert!(next_dispatchable(&reg, 2).is_empty());
    }

    #[test]
    fn claiming_returns_nothing_when_saturated() {
        let (mut reg, ids) = reg_with(4);
        reg.set_status(&ids[0], JobStatus::Downloading);
        reg.set_status(&ids[1], JobStatus::Processing);

        assert!(claim_next(&mut reg, 2).is_empty());
    }

    #[test]
    fn a_claim_marks_every_job_it_returns_probing() {
        let (mut reg, _) = reg_with(3);
        for id in claim_next(&mut reg, 3) {
            assert_eq!(reg.get(&id).unwrap().status, JobStatus::Probing);
        }
    }

    #[test]
    fn promotion_succeeds_for_a_job_this_run_claimed() {
        let (mut reg, _) = reg_with(1);
        let id = claim_next(&mut reg, 1).remove(0);

        assert!(promote_to_downloading(&mut reg, &id));
        assert_eq!(reg.get(&id).unwrap().status, JobStatus::Downloading);
    }

    // Spec section 4.4 / the cancel-between-spawn-and-attach window: a cancel
    // that lands before the promotion must not be overwritten by it.
    #[test]
    fn promotion_refuses_a_job_cancelled_after_the_claim() {
        let (mut reg, _) = reg_with(1);
        let id = claim_next(&mut reg, 1).remove(0);
        reg.cancel(&id);

        assert!(!promote_to_downloading(&mut reg, &id));
        assert_eq!(reg.get(&id).unwrap().status, JobStatus::Cancelled);
    }

    #[test]
    fn promotion_refuses_a_job_paused_after_the_claim() {
        let (mut reg, _) = reg_with(1);
        let id = claim_next(&mut reg, 1).remove(0);
        pause(&mut reg, &id);

        assert!(!promote_to_downloading(&mut reg, &id));
        assert_eq!(reg.get(&id).unwrap().status, JobStatus::Paused);
    }

    #[test]
    fn promotion_refuses_an_unknown_job() {
        let (mut reg, _) = reg_with(0);
        assert!(!promote_to_downloading(&mut reg, &"nonexistent".to_string()));
    }

    #[test]
    fn a_promoted_job_is_still_running() {
        let (mut reg, _) = reg_with(1);
        let id = claim_next(&mut reg, 1).remove(0);
        promote_to_downloading(&mut reg, &id);

        assert!(still_running(&reg, &id));
    }

    #[test]
    fn a_paused_download_is_no_longer_the_runs_to_report_on() {
        let (mut reg, _) = reg_with(1);
        let id = claim_next(&mut reg, 1).remove(0);
        promote_to_downloading(&mut reg, &id);
        pause(&mut reg, &id);

        assert!(!still_running(&reg, &id));
    }

    #[test]
    fn a_cancelled_download_is_no_longer_the_runs_to_report_on() {
        let (mut reg, _) = reg_with(1);
        let id = claim_next(&mut reg, 1).remove(0);
        promote_to_downloading(&mut reg, &id);
        reg.cancel(&id);

        assert!(!still_running(&reg, &id));
    }

    #[test]
    fn resuming_returns_a_job_to_the_queue() {
        let (mut reg, ids) = reg_with(1);
        pause(&mut reg, &ids[0]);
        resume(&mut reg, &ids[0]);
        assert_eq!(reg.get(&ids[0]).unwrap().status, JobStatus::Queued);
    }

    #[test]
    fn terminal_jobs_are_never_dispatched() {
        let (mut reg, ids) = reg_with(3);
        reg.set_status(&ids[0], JobStatus::Done);
        reg.set_status(&ids[1], JobStatus::Failed);
        assert_eq!(next_dispatchable(&reg, 4), vec![ids[2].clone()]);
    }

    // A finished job's real outcome must never be silently rewritten to
    // Paused by a stray pause() call.
    #[test]
    fn pausing_a_terminal_job_leaves_it_alone() {
        for terminal in [JobStatus::Done, JobStatus::Failed, JobStatus::Cancelled] {
            let (mut reg, ids) = reg_with(1);
            reg.set_status(&ids[0], terminal);
            pause(&mut reg, &ids[0]);
            assert_eq!(reg.get(&ids[0]).unwrap().status, terminal);
        }
    }

    #[test]
    fn pausing_and_resuming_a_missing_job_is_a_no_op_not_a_panic() {
        let mut reg = JobRegistry::new();
        let missing = "nonexistent".to_string();
        pause(&mut reg, &missing);
        resume(&mut reg, &missing);
        assert_eq!(reg.list().len(), 0);
    }
}
