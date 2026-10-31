import { component$ } from "@builder.io/qwik";
import type { PayrollRow, PayrollWeek } from "~/lib/bindings";
import { amountCell, DEBT_KINDS, periodLabel } from "./hr";

// Print layouts. They're hidden on screen (`hidden print:block`) and the
// screen content is hidden in print. The payroll prints on landscape
// Letter, payslips on portrait Letter, two to a sheet (`@page` rules in
// global.css).

const money = (c: number) =>
  (c / 100).toLocaleString("en-PH", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

const cell = "border border-black px-1.5 py-1";

/** The weekly payroll with a signature column, like the paper one. */
export const PayrollSheet = component$<{ p: PayrollWeek }>(({ p }) => {
  const rows = p.rows.filter((r) => r.worked);
  return (
    <div class="print-payroll text-[11pt] text-black">
      <h1 class="text-center text-[14pt] font-bold">PAYROLL</h1>
      <p class="mb-3 text-center text-[13pt] font-bold">{periodLabel(p.week)}</p>
      <table class="w-full border-collapse">
        <thead>
          <tr class="text-[10pt]">
            <th class={cell}>Employee</th>
            <th class={cell}>Rate</th>
            <th class={cell}># of D</th>
            <th class={cell}>Total</th>
            <th class={cell}>SSS/PI/PH</th>
            {DEBT_KINDS.map((k) => (
              <th key={k.kind} class={cell}>
                {k.label}
              </th>
            ))}
            <th class={cell}>Amount</th>
            <th class={[cell, "w-[22%]"]}>Signature</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.employee_id} class="h-[0.5in]">
              <td class={cell}>{r.name}</td>
              <td class={[cell, "text-right"]}>{money(r.daily_rate_c)}</td>
              <td class={[cell, "text-center"]}>{r.days}</td>
              <td class={[cell, "text-right"]}>{money(r.gross_c)}</td>
              <td class={[cell, "text-right"]}>{amountCell(r.contrib_c)}</td>
              {DEBT_KINDS.map((k) => (
                <td key={k.kind} class={[cell, "text-right"]}>
                  {amountCell(r[k.field])}
                </td>
              ))}
              <td class={[cell, "text-right"]}>{money(r.net_c)}</td>
              <td class={cell}></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
});

const slipCell = "border border-black px-2 py-0.5";

/** One payslip, laid out like the current paper one. */
export const Payslip = component$<{ company: string; week: string; r: PayrollRow }>(({ company, week, r }) => {
  const deductions: [string, number][] = [
    ["Philhealth", r.philhealth_c],
    ["Pag-Ibig", r.pagibig_c],
    ["SSS Loan", r.sss_el_c + r.sss_sl_c],
    ["Pag-Ibig Loan", r.pi_loan_c + r.pi_stl_c],
    ["Vale", r.vale_c],
  ];
  return (
    <div class="payslip text-[11pt] text-black">
      <div class="text-center font-bold leading-tight">
        <div class="text-[14pt] uppercase">{company}</div>
        <div class="text-[13pt]">PAYSLIP</div>
        <div>{periodLabel(week)}</div>
      </div>
      <div class="my-2 flex gap-10 pl-6 text-[12pt]">
        <span>Employee:</span>
        <span>{r.name}</span>
      </div>
      <table class="w-full border-collapse">
        <tbody>
          <tr>
            <td class={[slipCell, "text-center"]} colSpan={4}>
              Payments
            </td>
            <td class={[slipCell, "text-center"]} colSpan={2}>
              Deductions
            </td>
          </tr>
          <tr>
            <td class={slipCell}></td>
            <td class={[slipCell, "text-center"]}># of Days</td>
            <td class={[slipCell, "text-center"]}>Rate</td>
            <td class={[slipCell, "text-center"]}>Total</td>
            <td class={slipCell}></td>
            <td class={slipCell}>Amount</td>
          </tr>
          <tr>
            <td class={slipCell}>Basic Pay</td>
            <td class={[slipCell, "text-center"]}>{r.days}</td>
            <td class={[slipCell, "text-right"]}>{money(r.daily_rate_c)}</td>
            <td class={[slipCell, "text-right"]}>{money(r.gross_c)}</td>
            <td class={slipCell}>SSS</td>
            <td class={[slipCell, "text-right"]}>{amountCell(r.sss_c)}</td>
          </tr>
          {deductions.map(([label, c], i) => (
            <tr key={label}>
              {i === 0 && <td class="border-x border-black" colSpan={4} rowSpan={deductions.length}></td>}
              <td class={slipCell}>{label}</td>
              <td class={[slipCell, "text-right"]}>{amountCell(c)}</td>
            </tr>
          ))}
          <tr>
            <td class={slipCell}>Total</td>
            <td class={slipCell}></td>
            <td class={slipCell}></td>
            <td class={[slipCell, "text-right"]}>{money(r.gross_c)}</td>
            <td class={slipCell}>Total Deductions</td>
            <td class={[slipCell, "text-right"]}>{money(r.deductions_c)}</td>
          </tr>
        </tbody>
      </table>
      <div class="mt-3 flex justify-end gap-16 pr-2 text-[12pt] font-bold">
        <span>Net Pay</span>
        <span>{money(r.net_c)}</span>
      </div>
      <div class="mt-8 flex items-end gap-4 pl-6">
        <span>Received By:</span>
        <span class="w-[55%] border-b border-black"></span>
      </div>
    </div>
  );
});

/** Payslips, two to a Letter sheet with a cut line between. */
export const PayslipSheets = component$<{ company: string; week: string; rows: PayrollRow[] }>(
  ({ company, week, rows }) => {
    const pairs: PayrollRow[][] = [];
    for (let i = 0; i < rows.length; i += 2) pairs.push(rows.slice(i, i + 2));
    return (
      <div class="print-payslips">
        {pairs.map((pair, i) => (
          <div key={i} class={["flex h-[10.4in] flex-col", i < pairs.length - 1 && "break-after-page"]}>
            {pair.map((r, j) => (
              <div key={r.employee_id} class={["h-1/2 pt-2", j === 0 && pair.length > 1 && "border-b border-dashed border-gray-400"]}>
                <Payslip company={company} week={week} r={r} />
              </div>
            ))}
          </div>
        ))}
      </div>
    );
  },
);
