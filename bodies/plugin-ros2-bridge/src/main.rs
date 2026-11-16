//! `plugin-ros2-bridge` binary: verifies ROS 2 `Lyrical Luth` runtime availability and
//! exits with an explicit error when the ROS 2 installation or `ros2-rclrs` feature is missing.

#![forbid(unsafe_code)]

use plugin_ros2_bridge::check_ros2_runtime_available;

fn main() {
    if let Err(err) = check_ros2_runtime_available() {
        eprintln!("plugin-ros2-bridge: {err}");
        std::process::exit(1);
    }

    #[cfg(feature = "ros2-rclrs")]
    {
        if let Err(err) = plugin_ros2_bridge::rclrs_backend::init_ros2_node("monoagens_bridge") {
            eprintln!("plugin-ros2-bridge init failed: {err}");
            std::process::exit(1);
        }
    }
}
