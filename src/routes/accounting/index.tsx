// Accounting opens on Receivables. (DRs and invoices moved to Booking.)
import Receivables, { head as receivablesHead } from "./receivables";

export default Receivables;
export const head = receivablesHead;
