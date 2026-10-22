-- M5: master data. Clients, suppliers, locations, items with their cost
-- sheets, and per-client rates.
--
-- Percentages are integer basis points (20% = 2000). Money is centavos;
-- supplier prices are minor units of their currency. FX rates are decimal
-- text. Client and supplier codes live in doc_numbers (series CL, SUP).

-- Replicated ---------------------------------------------------------------

CREATE TABLE clients (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name           TEXT,
    contact_person TEXT,
    phone          TEXT,
    email          TEXT,
    tin            TEXT,
    address        TEXT,
    terms_days     INTEGER,
    margin_tier_bp INTEGER,
    credit_limit_c INTEGER,
    notes          TEXT,
    active         INTEGER
);

CREATE TABLE suppliers (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name           TEXT,
    country        TEXT,
    currency       TEXT,
    contact_person TEXT,
    phone          TEXT,
    email          TEXT,
    address        TEXT,
    notes          TEXT,
    active         INTEGER
);

CREATE TABLE locations (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name   TEXT,
    notes  TEXT,
    sort   INTEGER,
    active INTEGER
);

-- Cost-sheet charge types (Freight, Customs duty, ...). mode: per_unit | pct.
CREATE TABLE charge_types (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    name   TEXT,
    mode   TEXT,
    sort   INTEGER,
    active INTEGER
);

CREATE TABLE items (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    code              TEXT,
    name              TEXT,
    unit              TEXT,
    category          TEXT,
    supplier_id       TEXT,
    tracks_expiry     INTEGER,
    reorder_level     INTEGER,
    default_margin_bp INTEGER,
    notes             TEXT,
    active            INTEGER
);
CREATE INDEX items_code ON items (code);
CREATE INDEX items_supplier ON items (supplier_id);

-- One cost sheet per item. id = derived_id(item_id, "cost").
CREATE TABLE item_costs (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    item_id              TEXT,
    currency             TEXT,
    supplier_price_minor INTEGER,
    fx_rate              TEXT,
    notes                TEXT
);
CREATE INDEX item_costs_item ON item_costs (item_id);

-- One line per item and charge type. id = derived_id(item_id, "charge/{type}").
-- value: centavos per unit (per_unit) or basis points of the PHP base cost (pct).
-- A cleared line has value NULL.
CREATE TABLE item_cost_charges (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    item_id        TEXT,
    charge_type_id TEXT,
    mode           TEXT,
    value          INTEGER
);
CREATE INDEX item_cost_charges_item ON item_cost_charges (item_id);

-- Manual price per client and item, net of VAT. id = derived_id(client_id, item_id).
-- A cleared rate has price_c NULL.
CREATE TABLE client_item_rates (
    id TEXT PRIMARY KEY, _hlc INTEGER NOT NULL, _site TEXT NOT NULL, _vv TEXT NOT NULL,
    _deleted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, created_by TEXT, updated_by TEXT,
    client_id TEXT,
    item_id   TEXT,
    price_c   INTEGER,
    vat       INTEGER,
    notes     TEXT
);
CREATE INDEX client_item_rates_client ON client_item_rates (client_id);
CREATE INDEX client_item_rates_item ON client_item_rates (item_id);
