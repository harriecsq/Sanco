# HR module: plan

Status: agreed with the user on 2026-10-31. **Built 2026-10-31** (see "Build notes" at the end).

The HR module covers employees and salary only for now.

## 1. Screens

| Screen | Route | Purpose |
|---|---|---|
| Employees | `/hr/employees/` (profile: `?id=`) | List and profile |
| Attendance | `/hr/?week=` (the module's first tab) | Weekly grid of click-to-toggle boxes |
| Debts | `/hr/debts/` | Loans and vale, with balance and schedule |
| Payroll | `/hr/payroll/?week=` | Saturday salary table, post, print |

- Employees are separate from app users.
- `week` is always the Saturday's date (`YYYY-MM-DD`).

## 2. The week

- Work days are **Monday to Saturday**. There is no Sunday.
- Payday is the Saturday of that week.
- The period label reads "October 26 2026 to October 31 2026" (Monday to Saturday).
- "Today" is the date in Asia/Manila.

## 3. Attendance

- One box per active employee per day.
- Clicking cycles **white → green → red → yellow → white**.
- A legend is always shown above the grid:

  | Color | Meaning |
  |---|---|
  | White | Not marked |
  | Green | Present |
  | Red | Absent (optional reason) |
  | Yellow | Holiday (optional name) |

- Each day's header has a "Mark holiday for everyone" button.
- **A holiday is treated exactly like an absence:** not paid, and it doesn't count as working. The only difference is that it doesn't count toward the employee's **absences** total.
- **# of Days = green boxes only.**
- Row id `derived_id(employee, date)`, with `status` = present / absent / holiday / NULL (white).
  - Going back to white clears the status; the row is never deleted.
  - Two PCs clicking the same box merge, and the last click wins.
- Once the week's payroll is posted, its attendance is **locked**. Void the payroll to change it.

## 4. Employees

**Fields:**
- name
- position (optional)
- daily rate
- SSS, PhilHealth and Pag-IBIG per week (three fixed amounts, centavos allowed)
- start date
- active
- notes

Employees who leave are set inactive, never deleted. Rate changes affect unposted weeks only, because a posted payroll keeps a snapshot.

**The profile shows:**
- Details.
- **Current debts:** type, amount, paid, **balance**, this week's charge, last week, remit date and warnings. A "Show paid debts" toggle reveals old ones.
- **This year:** **absences** (red only), plus days present and holidays.
- **Payslip history:** each posted week with its net pay and a "Print payslip" button.

## 5. Debts

**Types:** PI Loan, PI STL, SSS EL, SSS SL, Vale. This is a fixed list, because each type is its own payroll column.

**Fields:**
- employee
- type
- amount
- start date (default today)
- **either** number of weeks **or** a typed weekly amount
- **remit date** (the debt must be fully paid by then)
- notes

**Money step: ₱0.05.** Debt amounts, weekly amounts and edited charges must be multiples of 5 centavos (₱478.50 and ₱288.25 are OK; ₱288.23 is not).
- A computed weekly amount is rounded **down** to the nearest ₱0.05.
- The last week takes the remainder.

**Weekly amount:**
- **Weeks entered:** weekly = amount ÷ weeks, rounded down to ₱0.05. The form shows "₱333.30/week, last week ₱333.40" live.
  - Example: ₱1,000 over 3 weeks gives ₱333.30, ₱333.30, ₱333.40.
- **Weekly amount typed:** weeks = ceil(amount ÷ weekly), and the last week takes the rest.
  - Example: ₱2,000 at ₱478.50 gives 4 × ₱478.50, then ₱86.00.

**When charging starts:** the first Saturday on or after the start date. A debt added Friday or Saturday is charged that same Saturday.

**Last week** = first Saturday + (weeks − 1) weeks.

**Remit warning:** shown if the last week falls after the remit date. It shows on the form, on the debts list and on the profile. It's a warning, not a block.

**Charge each Saturday:** nothing is stored; it is recalculated from the balance every week.
- No charge before the first Saturday, after the balance reaches zero, or in a week the employee didn't work (no green day).
- In the last week or later: the whole balance.
- Before the last week:
  - `floor_to_5c(balance ÷ weeks left including this one)`
  - with a typed weekly amount: the greater of that and the typed amount
  - never more than the balance

  This spreads anything skipped or reduced evenly over the remaining weeks.
- If the balance is still left after the last week, the debt is flagged **Overdue schedule**. It is also flagged **Past remit date** once that date passes.

**Balance** = amount − deductions from posted payrolls. It is a view (`v_debt_balance`), not stored.

**Statuses:** Active, Paid, Overdue schedule, Past remit date.

**Deleting a debt:** allowed only while nothing has been deducted from it. Otherwise, edit it (amount, weeks, remit date) and the schedule recalculates.

**Debts tab columns:** Employee, Type, Amount, Paid, **Balance**, This week, Last week, Remit date, Status. It has filters (employee, type, status) and hides paid debts by default.

## 6. Payroll

**Columns:** Employee | Rate | # of D | Total | SSS/PI/PH | PI Loan | PI STL | SSS EL | SSS SL | Vale | Amount

- Total = rate × green days.
- SSS/PI/PH = the three contributions added together. Hovering shows the split.
- Each loan column = the charges for that employee's debts of that type.
- Amount = Total − all deductions.
- A **week total** (the sum of every Amount, i.e. cash needed for payday) is shown on screen above the table. It is **not** printed.

**Employees who didn't work** (no green day) get no pay, no contributions and no deductions. They appear as a grey "No work this week" row and are left off the printouts.

**Draft (not posted):**
- Computed live from attendance, rates and debts.
- Debt cells are editable, in ₱0.05 steps, between 0 and the balance. Edits are saved per (debt, week): `debt_overrides`, `derived_id(debt, week)`, cleared rather than deleted. A reduced amount spreads over the following weeks through the charge rule.

**Over-deduction:**
- If deductions exceed Total, the row turns red: "Deductions exceed pay by ₱X. Reduce a loan."
- **Posting is blocked** until every red row is fixed.

**Post** (Approve permission):
- Saves a snapshot: the `payrolls` row (`derived_id("payroll", week)`), one `payroll_lines` row per employee (`derived_id(payroll, employee)`), and `payroll_debt_lines` (`derived_id(line, debt)`).
- Before posting, a warning lists any white boxes left in the week.

**Void:**
- Sets the status to voided and clears the debt lines to 0, so the balances return.
- Re-posting reuses the same rows.
- Because the id is derived from the week, the same week can't be posted twice on two PCs.

## 7. Printing

- Printing uses the Windows print dialog through `window.print()`, which also offers "Save as PDF".
- The company name comes from Settings → Company.
- Print CSS uses named `@page` rules.
- Printing is allowed only from a **posted** payroll.

**Payroll (all employees): Letter, landscape (forced).**
- Title **PAYROLL** and the period.
- Columns as in section 6, plus a **Signature** column.
- Rows are tall enough to sign, and empty cells are left blank.
- This matches the current Excel sheet. There is no totals row.

**Payslip (per employee):** this matches the current payslip.
- Company name, **PAYSLIP**, the period and the employee's name.
- **Payments:** Basic Pay, # of Days, Rate, Total.
- **Deductions:**
  - SSS
  - PhilHealth
  - Pag-IBIG
  - **SSS Loan** (= SSS EL + SSS SL)
  - **Pag-IBIG Loan** (= PI Loan + PI STL)
  - Vale
  - Total Deductions
- **Net Pay** and a **Received By** line.
- There's a print icon on each payroll row and on each posted week in the profile, which prints one slip per Letter page.
- "Print all payslips" prints **two per Letter sheet**, to cut in half.

## 8. Data

**Migration `0013_hr.sql`:** every table has the standard meta columns and is added to `registry.rs`.
- `employees`
- `attendance`
- `employee_debts`
- `debt_overrides`
- `payrolls`
- `payroll_lines`
- `payroll_debt_lines`

**Views:** `v_debt_balance`.

**Units:**
- Money is centavos (`i64`).
- Dates are `YYYY-MM-DD`.
- No stored derived values (balances, charges and the draft payroll are computed).

## 9. Code

- **Core logic:** `sanco-core/src/domain/hr.rs`. It holds the week math, the ₱0.05 rounding, the schedule and charge rule, the remit warning, the payroll calculation, and post/void. All writes go through `repo.rs`.
- **Tauri commands:** `src-tauri/src/commands/hr.rs`, each starting with `require(&state, Module::Hr, …)`.
- **Permissions:** a new `Module::Hr` (view / create / edit / delete / approve, where approve = post/void payroll). It is granted only to admins by default.
- **Sidebar:** an "HR" entry.
- **Dashboard issues:**
  - debts past their remit date with a balance left
  - debts with an overdue schedule
  - past Saturdays with no posted payroll

## 10. Tests (sanco-core)

- **First charge:** debt added Fri, Sat or Sun.
- **₱0.05 rounding and remainder:** ₱1,000 over 3 weeks; ₱2,000 at ₱478.50.
- **Rejecting amounts** that aren't multiples of 5 centavos.
- **Remit warning.**
- **Skipped week:** the missed amount is spread over the remaining weeks.
- **Override:** the reduced amount is spread; past the last week, the full balance is charged.
- **Holiday:** unpaid, doesn't count as working, and is not counted as an absence.
- **No-work week:** no contributions and no deductions.
- **Over-deduction:** blocks posting.
- **Post / void / re-post:** balances are correct after each step.
- **Registry and schema agree.**

Build order: schema and core with tests → Employees → Attendance → Debts → Payroll → printing.

## Build notes

- **Routes:** Attendance is the HR index (`/hr/`); Payroll, Debts and Employees are tabs next to it.
- **Last charge:** a computed weekly amount (₱1,000 over 3 weeks) gives ₱333.30, ₱333.30 and a last week of ₱333.40. This comes from the charge rule: on schedule it charges the weekly amount; behind schedule it charges the weekly amount plus the shortfall ÷ weeks left. It doesn't re-divide the balance every week, which would give ₱333.35 in week 2.
- **Typed weekly amount:** it is a minimum. After a skip, the catch-up is added on top.
- **Remit date:** optional, since a vale may have none.
- **Holidays and inactive employees:**
  - "Mark holiday for everyone" overwrites every active employee's box for that day.
  - "All ✓" only fills boxes that aren't marked yet.
  - Inactive employees still appear in a week where they have a mark.
- **Voiding and re-posting:**
  - Voiding only changes the payroll's status.
  - A re-post reuses the derived-id rows. Lines it leaves out get `included = 0`, and debt lines it leaves out get `amount_c = 0`.
  - Balances count only posted payrolls (`v_debt_paid`).
- **Dashboard:**
  - "Payroll this week" KPI.
  - Issues: payrolls not posted (past weeks with days present), debts past their remit date, debts behind schedule.
- **Printing:**
  - `window.print()` with named `@page` rules in `global.css`: `payroll` is Letter landscape, `payslip` is Letter portrait.
  - The app shell is hidden in print (`print:hidden`).
  - Checked as PDFs in headless Chromium against the Oct 19–24 sheet: same amounts.
- **Schema version:** this adds migration 13. PCs on a build without HR can't sync with PCs on one with it, so install the same build everywhere.

