# Clockeroo 

Another stupid tui project I made for fun since I keep burning rice.

![Demo](quick-demo.gif)

## Install

```bash
cargo install --path .
```

## Usage

### Timer
```bash
clockeroo timer 20m      # 20 minutes
clockeroo timer 1h30m    # 1 hour 30 minutes  
clockeroo timer 90       # 90 seconds
```

### Stopwatch
```bash
clockeroo stopwatch start
# Press 's' to stop, 'q' to quit
```

### Alarm
```bash
clockeroo alarm 7:30am   # Morning alarm
clockeroo alarm 14:30    # 24-hour format
```

## Controls

- `q` or `Ctrl-C` - Exit
- `s` - Stop stopwatch (stopwatch mode only)

## Features

- Clean ASCII art UI
- Desktop notifications (requires notification daemon on Linux)
- Sound alert

Not sure if this will be helpful for you but there ya go :)
