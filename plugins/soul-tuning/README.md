# Soul Tuning plugin

A plugin of MonoAgens: it follows the rules every plugin follows ([spec.md](../../docs/spec.md)
*Plugins*). This file holds only what is specific to Soul Tuning.

Tunes expression, never identity; only the owner changes it (absolute line 1).
- **Modes** (daily, professional, …): standing rules in `<self>`, knob offsets within bounds,
  the `/mode` command.
- **Example conversations**: short exchanges in the stable prompt, in a fixed order.
- Who a person is to the soul ("a customer; help with orders") is not here: it is each role's
  and person's `context` in `roles.py`.
