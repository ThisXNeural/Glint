`i-slint-backend-winit` 1.18.1 with one change in `frame_throttle.rs`: a
monitor refresh rate of 0 Hz falls back to 60 Hz instead of dividing by zero.
