# Roadmap

## Planned

Mirrored from `hdmi-hal`: each item lands there first and here in the same release, with
the same shape. See `hdmi-hal`'s
[`doc/roadmap.md`](https://github.com/ashlar-display/hdmi-hal/blob/main/doc/roadmap.md)
for the details.

- **Further PHY configuration** — lane mapping, TMDS pre-emphasis and swing,
  spread-spectrum clocking and power states, each decided when the layer that needs it is
  built. As trait methods with default implementations they are not breaking changes; as
  required methods they would be.
- **CEC line trait** — a single-wire CEC bus access primitive, when CEC work begins.
