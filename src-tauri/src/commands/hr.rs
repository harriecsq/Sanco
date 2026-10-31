//! HR: employees, attendance, debts and the weekly payroll. Salaries are
//! sensitive, so every HR screen needs the HR module itself.

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::hr::{
    self, AttendanceStatus, AttendanceWeek, DebtInfo, DebtInput, DebtPlan, DebtPlanInput, Employee,
    EmployeeInput, EmployeeProfile, PayrollWeek,
};
use sanco_core::time::manila_date_string;
use tauri::State;

use crate::commands::{blocking, require, require_save};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_employees(state: State<'_, AppState>) -> Result<Vec<Employee>, String> {
    let (db, _) = require(&state, Module::Hr, Action::View).await?;
    blocking(move || db.read(hr::list_employees)).await
}

/// An employee with their debts, this year's attendance and payslips.
#[tauri::command]
#[specta::specta]
pub async fn employee_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<EmployeeProfile>, String> {
    let (db, _) = require(&state, Module::Hr, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| hr::employee_profile(c, &id, &today))).await
}

/// Adds or saves an employee. Returns their id.
#[tauri::command]
#[specta::specta]
pub async fn save_employee(
    state: State<'_, AppState>,
    employee: EmployeeInput,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Hr, employee.id.is_none()).await?;
    blocking(move || db.write(user.actor(), move |tx| hr::save_employee(tx, &employee))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_employee(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Hr, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| hr::delete_employee(tx, &id))).await
}

/// Monday to Saturday of a week (given by its Saturday).
#[tauri::command]
#[specta::specta]
pub async fn attendance_week(
    state: State<'_, AppState>,
    week: String,
) -> Result<AttendanceWeek, String> {
    let (db, _) = require(&state, Module::Hr, Action::View).await?;
    blocking(move || db.read(|c| hr::attendance_week(c, &week))).await
}

/// Sets one attendance box. `status` null = not marked.
#[tauri::command]
#[specta::specta]
pub async fn set_attendance(
    state: State<'_, AppState>,
    employee_id: String,
    date: String,
    status: Option<AttendanceStatus>,
    note: String,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Hr, Action::Edit).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            hr::set_attendance(tx, &employee_id, &date, status, &note)
        })
    })
    .await
}

/// Marks a day for every active employee (or only the boxes not marked).
#[tauri::command]
#[specta::specta]
pub async fn mark_attendance_day(
    state: State<'_, AppState>,
    date: String,
    status: AttendanceStatus,
    note: String,
    only_unmarked: bool,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Hr, Action::Edit).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            hr::mark_day(tx, &date, status, &note, only_unmarked)
        })
    })
    .await
}

/// Debts of one employee, or everyone's.
#[tauri::command]
#[specta::specta]
pub async fn list_debts(
    state: State<'_, AppState>,
    employee_id: Option<String>,
) -> Result<Vec<DebtInfo>, String> {
    let (db, _) = require(&state, Module::Hr, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| hr::list_debts(c, employee_id.as_deref(), &today))).await
}

/// What a debt's amount and weeks (or weekly amount) work out to.
#[tauri::command]
#[specta::specta]
pub async fn debt_plan(
    state: State<'_, AppState>,
    plan: DebtPlanInput,
) -> Result<DebtPlan, String> {
    require(&state, Module::Hr, Action::View).await?;
    hr::debt_plan(&plan).map_err(|e| e.to_string())
}

/// Adds or saves a debt. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_debt(state: State<'_, AppState>, debt: DebtInput) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Hr, debt.id.is_none()).await?;
    blocking(move || db.write(user.actor(), move |tx| hr::save_debt(tx, &debt))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_debt(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Hr, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| hr::delete_debt(tx, &id))).await
}

/// Changes a debt's charge on a draft payroll; `amount_c` null = back to
/// the schedule.
#[tauri::command]
#[specta::specta]
pub async fn set_debt_charge(
    state: State<'_, AppState>,
    debt_id: String,
    week: String,
    amount_c: Option<i64>,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Hr, Action::Edit).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            hr::set_debt_charge(tx, &debt_id, &week, amount_c)
        })
    })
    .await
}

/// A week's payroll: posted, or computed live. `week` empty = this week.
#[tauri::command]
#[specta::specta]
pub async fn payroll_week(state: State<'_, AppState>, week: String) -> Result<PayrollWeek, String> {
    let (db, _) = require(&state, Module::Hr, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    let week = if week.trim().is_empty() {
        hr::week_of(&today).unwrap_or_default()
    } else {
        week
    };
    blocking(move || db.read(|c| hr::payroll_week(c, &week, &today))).await
}

#[tauri::command]
#[specta::specta]
pub async fn post_payroll(state: State<'_, AppState>, week: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Hr, Action::Approve).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.write(user.actor(), move |tx| hr::post_payroll(tx, &week, &today))).await
}

#[tauri::command]
#[specta::specta]
pub async fn void_payroll(state: State<'_, AppState>, week: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Hr, Action::Approve).await?;
    blocking(move || db.write(user.actor(), move |tx| hr::void_payroll(tx, &week))).await
}
