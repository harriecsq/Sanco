mod common;

use common::TestPeer;
use sanco_core::domain::auth::{Action, Module, Perms};
use sanco_core::domain::dashboard;
use sanco_core::domain::hr::{
    self, AttendanceStatus, DebtInput, DebtKind, DebtPlanInput, DebtStatus, EmployeeInput,
    PayrollStatus,
};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::{Actor, Db, Error};

/// Saturdays.
const W1: &str = "2026-10-31";
const W2: &str = "2026-11-07";
const W3: &str = "2026-11-14";
const W4: &str = "2026-11-21";

fn setup(db: &Db) -> Actor {
    users::create_first_admin(
        db,
        NewUser {
            username: "ana".into(),
            display_name: "Ana".into(),
            password: "secret1".into(),
            role_ids: vec![],
        },
    )
    .unwrap()
    .actor()
}

fn invalid<T: std::fmt::Debug>(r: Result<T, Error>) -> String {
    match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

/// Aquilino from the payslip: ₱760 a day, SSS 187.50, PhilHealth 73.25,
/// Pag-IBIG 50.00.
fn add_employee(db: &Db, actor: &Actor, name: &str) -> String {
    let input = EmployeeInput {
        id: None,
        name: name.into(),
        position: String::new(),
        daily_rate_c: 76_000,
        sss_c: 18_750,
        philhealth_c: 7_325,
        pagibig_c: 5_000,
        start_date: None,
        active: true,
        notes: String::new(),
    };
    db.write(actor.clone(), move |tx| hr::save_employee(tx, &input))
        .unwrap()
}

fn debt(employee: &str, kind: DebtKind, amount_c: i64, start: &str, weeks: i64) -> DebtInput {
    DebtInput {
        id: None,
        employee_id: employee.into(),
        kind,
        amount_c,
        start_date: start.into(),
        weeks: Some(weeks),
        weekly_c: None,
        remit_date: None,
        notes: String::new(),
    }
}

fn add_debt(db: &Db, actor: &Actor, input: DebtInput) -> String {
    db.write(actor.clone(), move |tx| hr::save_debt(tx, &input))
        .unwrap()
}

fn mark(db: &Db, actor: &Actor, e: &str, week: &str, statuses: [Option<AttendanceStatus>; 6]) {
    let days = hr::week_days(week);
    let e = e.to_string();
    db.write(actor.clone(), move |tx| {
        for (d, s) in days.iter().zip(statuses) {
            hr::set_attendance(tx, &e, d, s, "")?;
        }
        Ok(())
    })
    .unwrap();
}

const P: Option<AttendanceStatus> = Some(AttendanceStatus::Present);
const A: Option<AttendanceStatus> = Some(AttendanceStatus::Absent);
const H: Option<AttendanceStatus> = Some(AttendanceStatus::Holiday);

fn full_week(db: &Db, actor: &Actor, e: &str, week: &str) {
    mark(db, actor, e, week, [P; 6]);
}

fn post(db: &Db, actor: &Actor, week: &str) -> Result<(), Error> {
    let week = week.to_string();
    db.write(actor.clone(), move |tx| hr::post_payroll(tx, &week, &week))
}

fn payroll(db: &Db, week: &str) -> hr::PayrollWeek {
    db.read(|c| hr::payroll_week(c, week, week)).unwrap()
}

fn charges(db: &Db, week: &str, debt: &str) -> i64 {
    payroll(db, week)
        .rows
        .iter()
        .flat_map(|r| r.debts.iter())
        .filter(|d| d.debt_id == debt)
        .map(|d| d.amount_c)
        .sum()
}

fn balance(db: &Db, debt: &str) -> i64 {
    db.read(|c| hr::list_debts(c, None, W1))
        .unwrap()
        .into_iter()
        .find(|d| d.id == debt)
        .unwrap()
        .balance_c
}

fn plan(
    amount_c: i64,
    start: &str,
    weeks: Option<i64>,
    weekly_c: Option<i64>,
) -> Result<hr::DebtPlan, Error> {
    hr::debt_plan(&DebtPlanInput {
        amount_c,
        start_date: start.into(),
        weeks,
        weekly_c,
        remit_date: Some("2026-11-20".into()),
    })
}

#[test]
fn debt_plans_round_down_to_five_centavos() {
    // ₱1,000 over 3 weeks: ₱333.30, ₱333.30, ₱333.40.
    let p = plan(100_000, W1, Some(3), None).unwrap();
    assert_eq!((p.weeks, p.weekly_c, p.last_c), (3, 33_330, 33_340));
    // A typed ₱478.50 on ₱2,000: four of them, then ₱86.00.
    let p = plan(200_000, W1, None, Some(47_850)).unwrap();
    assert_eq!((p.weeks, p.weekly_c, p.last_c), (5, 47_850, 8_600));
    // Steps of ₱0.05 only.
    assert!(invalid(plan(28_823, W1, Some(2), None)).contains("₱0.05"));
    assert!(invalid(plan(100_000, W1, None, Some(47_853))).contains("₱0.05"));
    assert!(invalid(plan(100_000, W1, None, None)).contains("weeks"));
}

#[test]
fn charging_starts_the_saturday_on_or_after_the_start() {
    for (start, first) in [
        ("2026-10-30", W1), // Friday: this Saturday
        ("2026-10-31", W1), // Saturday: today
        ("2026-11-01", W2), // Sunday: next Saturday
    ] {
        assert_eq!(
            plan(100_000, start, Some(2), None).unwrap().first_week,
            first
        );
    }
    // 3 weeks from Oct 31 end Nov 14: fine. 4 end Nov 21: after Nov 20.
    assert!(!plan(100_000, W1, Some(3), None).unwrap().remit_warning);
    let p = plan(100_000, W1, Some(4), None).unwrap();
    assert_eq!(p.last_week, W4);
    assert!(p.remit_warning);
}

#[test]
fn payroll_matches_the_payslip() {
    let tmp = tempfile::tempdir().unwrap();
    let a = TestPeer::new(tmp.path(), "a", 1);
    let actor = setup(&a.db);
    let e = add_employee(&a.db, &actor, "Aquilino Aguillon");
    // A ₱1,914 Pag-IBIG loan at ₱478.50 a week.
    let loan = add_debt(
        &a.db,
        &actor,
        debt(&e, DebtKind::PiLoan, 191_400, "2026-10-30", 4),
    );
    full_week(&a.db, &actor, &e, W1);

    let p = payroll(&a.db, W1);
    assert_eq!(p.status, PayrollStatus::Draft);
    assert_eq!(p.monday, "2026-10-26");
    let r = &p.rows[0];
    assert_eq!((r.days, r.gross_c), (6, 456_000));
    assert_eq!(r.contrib_c, 31_075);
    assert_eq!(r.pi_loan_c, 47_850);
    assert_eq!(r.deductions_c, 78_925);
    assert_eq!(r.net_c, 377_075);
    assert_eq!(p.total_net_c, 377_075);
    assert!(p.can_post);

    post(&a.db, &actor, W1).unwrap();
    assert_eq!(balance(&a.db, &loan), 191_400 - 47_850);
    let p = payroll(&a.db, W1);
    assert_eq!(p.status, PayrollStatus::Posted);
    assert_eq!(p.rows[0].net_c, 377_075);
    assert!(!p.can_post);
    assert!(invalid(post(&a.db, &actor, W1)).contains("already posted"));

    // Attendance is locked until the payroll is voided.
    let (e2, d) = (e.clone(), hr::week_days(W1)[0].clone());
    let set = |status| {
        let (e2, d) = (e2.clone(), d.clone());
        a.db.write(actor.clone(), move |tx| {
            hr::set_attendance(tx, &e2, &d, status, "sick")
        })
    };
    assert!(invalid(set(A)).contains("posted"));
    a.db.write(actor.clone(), |tx| hr::void_payroll(tx, W1))
        .unwrap();
    assert_eq!(balance(&a.db, &loan), 191_400);
    set(A).unwrap();
    // Re-posted with 5 days.
    post(&a.db, &actor, W1).unwrap();
    let p = payroll(&a.db, W1);
    assert_eq!(p.rows[0].days, 5);
    assert_eq!(p.rows[0].absent, 1);
    assert_eq!(balance(&a.db, &loan), 191_400 - 47_850);
}

#[test]
fn holidays_are_unpaid_but_not_absences() {
    let tmp = tempfile::tempdir().unwrap();
    let a = TestPeer::new(tmp.path(), "a", 1);
    let actor = setup(&a.db);
    let e = add_employee(&a.db, &actor, "Julito Geronda");
    let vale = add_debt(&a.db, &actor, debt(&e, DebtKind::Vale, 100_000, W1, 2));

    // A week of only holidays and absences: nothing at all.
    mark(&a.db, &actor, &e, W1, [H, H, A, None, None, None]);
    let r = payroll(&a.db, W1).rows.remove(0);
    assert!(!r.worked);
    assert_eq!((r.gross_c, r.contrib_c, r.vale_c, r.net_c), (0, 0, 0, 0));
    assert_eq!((r.absent, r.holidays, r.unmarked), (1, 2, 3));

    // One day present: paid for one day, with contributions and the vale.
    mark(&a.db, &actor, &e, W1, [H, H, A, P, None, None]);
    let r = payroll(&a.db, W1).rows.remove(0);
    assert_eq!(r.gross_c, 76_000);
    assert_eq!(r.contrib_c, 31_075);
    assert_eq!(charges(&a.db, W1, &vale), 50_000);
    // ₱760 − ₱310.75 − ₱500 is negative: posting is blocked.
    let p = payroll(&a.db, W1);
    assert_eq!(p.over, vec!["Julito Geronda".to_string()]);
    assert!(!p.can_post);
    assert!(invalid(post(&a.db, &actor, W1)).contains("reduce a loan"));

    // Absences this year count absences, not holidays.
    let profile =
        a.db.read(|c| hr::employee_profile(c, &e, W1))
            .unwrap()
            .unwrap();
    assert_eq!(
        (profile.present, profile.absent, profile.holidays),
        (1, 1, 2)
    );
}

#[test]
fn skipped_and_reduced_weeks_spread_over_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let a = TestPeer::new(tmp.path(), "a", 1);
    let actor = setup(&a.db);
    let e = add_employee(&a.db, &actor, "Fernando Allas");
    // ₱1,000 over 4 weeks from Oct 31: ₱250 a week.
    let loan = add_debt(&a.db, &actor, debt(&e, DebtKind::SssSl, 100_000, W1, 4));

    full_week(&a.db, &actor, &e, W1);
    post(&a.db, &actor, W1).unwrap();
    // Week 2 not worked: nothing charged.
    mark(&a.db, &actor, &e, W2, [A; 6]);
    assert_eq!(charges(&a.db, W2, &loan), 0);
    // Week 3 catches up: ₱750 over the 2 weeks left.
    full_week(&a.db, &actor, &e, W3);
    assert_eq!(charges(&a.db, W3, &loan), 37_500);
    post(&a.db, &actor, W3).unwrap();

    // Week 4 (the last) is reduced to ₱200; the rest comes after.
    full_week(&a.db, &actor, &e, W4);
    assert_eq!(charges(&a.db, W4, &loan), 37_500);
    let l = loan.clone();
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| {
            hr::set_debt_charge(tx, &l, W4, Some(20_003))
        }))
        .contains("₱0.05")
    );
    let l = loan.clone();
    a.db.write(actor.clone(), move |tx| {
        hr::set_debt_charge(tx, &l, W4, Some(20_000))
    })
    .unwrap();
    assert_eq!(charges(&a.db, W4, &loan), 20_000);
    post(&a.db, &actor, W4).unwrap();
    assert_eq!(balance(&a.db, &loan), 17_500);

    // Past its last week with a balance: overdue, and charged in full.
    let d =
        a.db.read(|c| hr::list_debts(c, None, "2026-11-23"))
            .unwrap()
            .remove(0);
    assert_eq!(d.status, DebtStatus::OverdueSchedule);
    assert_eq!(d.this_week_c, 17_500);
    let admin = Perms::admin();
    assert!(admin.can(Module::Hr, Action::Approve));
    let dash =
        a.db.read(|c| dashboard::summary(c, &admin, "2026-11-23"))
            .unwrap();
    let keys: Vec<_> = dash.issues.iter().map(|i| i.key.as_str()).collect();
    assert!(keys.contains(&"debt_overdue"));
    // Week 2 has no days present, so nothing is waiting to be posted.
    assert!(!keys.contains(&"payroll_unposted"));
}

#[test]
fn typed_weekly_amounts_are_a_minimum() {
    let tmp = tempfile::tempdir().unwrap();
    let a = TestPeer::new(tmp.path(), "a", 1);
    let actor = setup(&a.db);
    let e = add_employee(&a.db, &actor, "Arniel Demain");
    let mut input = debt(&e, DebtKind::PiLoan, 200_000, W1, 0);
    input.weeks = None;
    input.weekly_c = Some(47_850);
    let loan = add_debt(&a.db, &actor, input);
    let d =
        a.db.read(|c| hr::list_debts(c, None, W1))
            .unwrap()
            .remove(0);
    assert!(d.weekly_typed);
    assert_eq!((d.weeks, d.weekly_c, d.last_c), (5, 47_850, 8_600));

    full_week(&a.db, &actor, &e, W1);
    assert_eq!(charges(&a.db, W1, &loan), 47_850);
    post(&a.db, &actor, W1).unwrap();
    // A week skipped: ₱1,521.50 over the 3 weeks left from Nov 14 is more
    // than ₱478.50 a week.
    full_week(&a.db, &actor, &e, W3);
    assert_eq!(charges(&a.db, W3, &loan), 47_850 + 15_950);
}

#[test]
fn debts_with_deductions_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let a = TestPeer::new(tmp.path(), "a", 1);
    let actor = setup(&a.db);
    let e = add_employee(&a.db, &actor, "Gerard Abron");
    let loan = add_debt(&a.db, &actor, debt(&e, DebtKind::SssEl, 100_000, W1, 4));
    full_week(&a.db, &actor, &e, W1);
    post(&a.db, &actor, W1).unwrap();

    let l = loan.clone();
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| hr::delete_debt(tx, &l)))
            .contains("can't be deleted")
    );
    let mut input = debt(&e, DebtKind::SssEl, 20_000, W1, 4);
    input.id = Some(loan.clone());
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| hr::save_debt(tx, &input)))
            .contains("can't be less")
    );
    let e2 = e.clone();
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| hr::delete_employee(tx, &e2)))
            .contains("inactive")
    );
    let profile =
        a.db.read(|c| hr::employee_profile(c, &e, W1))
            .unwrap()
            .unwrap();
    assert_eq!(profile.employee.debt_balance_c, 75_000);
    assert_eq!(profile.payslips.len(), 1);
    assert_eq!(profile.payslips[0].week, W1);
}

#[test]
fn staff_roles_do_not_get_hr() {
    assert!(!Perms::staff().can(Module::Hr, Action::View));
}
