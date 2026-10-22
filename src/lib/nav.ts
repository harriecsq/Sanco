import type { Module } from "./bindings";

export interface NavItem {
  module: Module;
  href: string;
  label: string;
  /** SVG path (24×24, stroked). */
  icon: string;
}

export const NAV: NavItem[] = [
  { module: "dashboard", href: "/", label: "Dashboard", icon: "M3 11.5 12 4l9 7.5M5.5 9.5V20h4.5v-6h4v6h4.5V9.5" },
  { module: "bd", href: "/bd/", label: "Business Dev", icon: "M4 5h16v11H9l-5 4z M8 9.5h8 M8 12.5h5" },
  { module: "bookings", href: "/bookings/", label: "Booking", icon: "M9 3.5h6v3H9z M7 5H5v15.5h14V5h-2 M8.5 11h7 M8.5 15h5" },
  { module: "accounting", href: "/accounting/", label: "Accounting", icon: "M4 6h16v12H4z M4 10h16 M8 14.5h3" },
  { module: "items", href: "/items/", label: "Items", icon: "M3.5 7.5 12 3.5l8.5 4-8.5 4z M3.5 7.5v9l8.5 4 8.5-4v-9 M12 11.5v9" },
  { module: "inventory", href: "/inventory/", label: "Inventory", icon: "M3 20.5V9l9-5.5L21 9v11.5 M7 20.5v-8h10v8 M7 16.5h10" },
  { module: "orders", href: "/orders/", label: "Order", icon: "M2.5 6h11.5v10H2.5z M14 9.5h4l3.5 3.5v3H14 M6.5 19.5a2 2 0 1 0 0-4 2 2 0 0 0 0 4z M17 19.5a2 2 0 1 0 0-4 2 2 0 0 0 0 4z" },
  { module: "clients", href: "/clients/", label: "Clients", icon: "M15.5 19.5v-1.5a3.5 3.5 0 0 0-3.5-3.5H6a3.5 3.5 0 0 0-3.5 3.5v1.5 M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z M21.5 19.5v-1.5a3.5 3.5 0 0 0-2.5-3.4 M15.5 4.2a3.5 3.5 0 0 1 0 6.6" },
  { module: "reports", href: "/reports/", label: "Reports", icon: "M3.5 20.5h17 M6.5 16.5v-5 M11 16.5v-10 M15.5 16.5v-7 M20 16.5v-3" },
  { module: "settings", href: "/settings/", label: "Settings", icon: "M4 6.5h9 M17 6.5h3 M4 12h3 M11 12h9 M4 17.5h11 M19 17.5h1 M15 4.5v4 M9 10v4 M17 15.5v4" },
];

