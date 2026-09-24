# Beauty shots

Every ten seconds of real time, the client takes a photograph through the ship's own telescope
of something worth looking at, and shows it in a square beside the map's. Code is
`lc_client::beauty`.

Status: **experimental**, off by default. The telescope window's *Beauty shots* box turns it
on, and so does `--beauty`.

## What is photographed

Everything that applies now, in turn, one per shot:

| subject | when | the shot |
|---|---|---|
| star | a stare, or a watch's current target | the star in a patch of sky around it, with diffraction spikes |
| surveyed body | a system survey, once it has measured a body | that body, framed |
| the survey's star | a survey that has measured nothing yet, or whose star is not this system's | as a star |
| field | a sweep | the field being exposed now, at the sweep's own field size |
| approach | flying to somewhere a body is within fifty of its radii of | that body, framed |
| destination | flying to somewhere with no body there, which across the gap is a star | the patch of sky ahead |
| whole | held by a body other than the star | the body, framed |
| horizon | the same, within thirty of its radii | along the limb on the star's side, looking just over it |
| below | the same | straight down |

The horizon and the ground below are framed by **length, not angle**: a stretch of limb and a
patch of ground a fixed share of the body's radius across, about 730 and 570 km on Earth. A
higher orbit is therefore a longer lens on the same scene. Straight down, the photograph from
twenty radii matches the one from low orbit; the horizon keeps its size but not its perspective,
because from far out the limb is seen edge-on rather than across the ground in front of it.

Rotating rather than ranking was a choice: a survey that runs for days would otherwise hold the
square on one kind of picture for all of them. `--beauty-kind <kind>` holds the rotation on one
kind for a photograph of the client, with the kinds named as in `Subject::kind`.

A photograph is taken from the eye, which is a boom's length behind the hull. Watching from
another craft pauses the shots: the subjects are this ship's, and the eye is not.

## One scene, two cameras

The telescope is a second `Camera3d` at the render origin, drawing the sky's own layer into an
image, active for one frame in ten seconds. There is no second copy of the world. What the two
views cannot share is handled where it lives:

- **A star's size is in pixels.** The starfield's radii are converted from pixels at the sky
  camera's scale, so through a narrow field every star would be a disc. The uniform carries the
  scale they were converted at (`drawn_rad_per_px`) and the shader rescales by its own view's,
  from `clip_from_view` and the viewport. The sky's own view comes out at exactly one.
- **So is its exposure.** The window is placed for the whole sky, and near a star everything
  interstellar is twenty stops under it. The shot camera carries a Bevy `Exposure`, which the
  starfield reads as a ratio to the sky's (`drawn_exposure`). A star shot is metered on its star,
  and a field or the sky ahead on the brightest star in it. Nothing else reads that ratio.
- **A body the sky still draws as a point.** `resolved::update_resolved` spawns a sphere for the
  shot's subject when the *shot* resolves it, on `SHOT_LAYER`, which only the telescope draws.
  That sphere gets a material of its own, metered for itself (`resolved::metered_for`). The
  surface shader's window is logarithmic, so an exposure applied after it cannot bring back a
  disc it has already clipped. A body the sky resolves too keeps the sky's sphere and exposure.
- **The ship itself.** Hulls, plumes and the haze composite are on `SKY_ONLY_LAYER`, which the
  sky's camera draws and the telescope does not. The haze is therefore missing from photographs.

## What distance does to a picture

The shot is drawn at as many pixels as the optics can fill: its field divided by the telescope's
diffraction limit, up to what the square shows. A field too small to hold twelve resolution
elements is widened until it does. So a far planet comes out as a few soft pixels upscaled, and
sharpens as the ship closes. There are no distance bands; this is the telescope's
`resolution_rad`. For a one square meter mirror the limit is about 0.6 microradians, so a planet
framed from inside its own system is sharp and one framed from a light-year away is a blob.

## What is not done

- Spikes are drawn by the interface over the photograph, at the middle of the frame, and only
  for a star shot. A field's stars have none.
- The band mapping is the view's. A Hubble palette per shot would need the starfield's band
  matrix per view, which is the same kind of change as the exposure and has not been made.
- Nothing is kept. A gallery of past shots would need the image copied out before it is drawn
  over, and a place to put them.
