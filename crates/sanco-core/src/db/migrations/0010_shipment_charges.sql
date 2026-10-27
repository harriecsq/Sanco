-- Cost-sheet charges for a whole shipment (item_cost_charges.mode and
-- charge_types.mode = 'shipment'): the line's value is the PHP total for the
-- shipment, divided by the cost sheet's shipment quantity for the per-unit
-- amount. NULL while no shipment charge needs it.

ALTER TABLE item_costs ADD COLUMN shipment_qty INTEGER;
