# bevy_openusd

A live [OpenUSD](https://openusd.org) editor on [Bevy](https://bevy.org).
`crates/usd_bevy` is the library, `usdview` is the viewer.

![Kitchen set](media/kitchen.png)

| ![Tractor](media/tractor.png) | ![Franka Panda](media/franka.png) |
| --- | --- |

## Run

```sh
direnv allow
make run --args path/to/scene.usd
```

## Used by

[gearbox](https://github.com/robolibs/gearbox), a farm machine and robot simulator
built on `usd_bevy`. Every machine is a USD file.

![Fendt with a disc harrow](media/gearbox-fendt.jpg)

| ![Field robots](media/gearbox-robots.jpg) | ![Gearbox machine pane](media/gearbox-ui.jpg) |
| --- | --- |

Supported by [Wageningen University & Research](https://www.wur.nl/). MIT licensed.
