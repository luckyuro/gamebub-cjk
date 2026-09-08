<p align="center">
  <a href="https://gamebub.net/">
    <img src="./assets/logo.svg" width="480" alt="Game Bub logo">
  </a>
</p>

[![Game Bub trailer](./assets/video-poster.jpg)](https://www.youtube.com/watch?v=f16E5J6qljw)

**Game Bub** is an open-source FPGA retro emulation handheld, with support for Game Boy, Game Boy Color, and Game Boy Advance games.

You can buy your own, prebuilt Game Bub from **[Crowd Supply](https://www.crowdsupply.com/second-bedroom/game-bub)**!

You can find the [user guide and documentation here](https://docs.gamebub.net/).

Want to chat about Game Bub? Feel free to [join the Discord](https://discord.gg/T5xrYpMfN7).

Check out the [announcement blog post](https://eli.lipsitz.net/posts/introducing-gamebub/) for an in-depth look at the development process!


## Features
* Play physical Game Boy / Color / Advance cartridges
* Load and play ROM files from a microSD card (with built-in support for rumble, clock, accelerometer, gyroscope)
* Multiplayer link cable functionality
* Custom, from-scratch Game Boy and Game Boy Advance FPGA cores with great game compatibility
* 14+ hour battery life 
* Video output to TV or monitor via Game Bub Dock
* Extensible hardware, designed for future improvements

## Building

Building a Game Bub handheld requires manufacturing PCBs, 3D printing the shell and buttons, and assembling components from a variety of sources. For information on manufacturing and assembling your own, see [here](https://github.com/elipsitz/gamebub/blob/v0.1/docs/building.md). Note that this information is for the older, vertical revision 2. Updated building information will be available in the Game Bub Docs soon.

For other inquiries, contact us at support@gamebub.net.

## Architecture

For an in-depth description of the project architecture, as of revision 2, see [here](https://github.com/elipsitz/gamebub/blob/v0.1/docs/architecture.md).

The Game Bub handheld consists of a Xilinx XC7A100T FPGA to do the main emulation and I/O, and an ESP32-S3 microcontroller to do auxiliary tasks (configuring the FPGA, rendering the UI, loading ROM files from a microSD card and sending it to the FPGA).

### Directory Structure

* `fpga`: FPGA source code (HDL), written in [Chisel](https://github.com/chipsalliance/chisel)
* `firmware`: Microcontroller firmware

## License

Unless otherwise specified:

* Firmware (in `firmware/`) and scripts (in `scripts/`) are licensed under GPLv3 (`GPL-3.0-only`).
* FPGA source code (in `fpga/`) is licensed under the CERN Open Hardware License Version 2 - Strongly Reciprocal (`CERN-OHL-S-2.0`)
* PCB (schematic and layout), mechanical, and hardware design files are licensed under the CERN Open Hardware License Version 2 - Strongly Reciprocal (`CERN-OHL-S-2.0`)

At a high level, this means that you can copy, share, and modify the source code, as long as you provide proper attribution and share your source code / design files with the same license. However, the "Game Bub" name and logo are trademarked, and you may not use them for your product without permission.

## ROM 列表编译选项

在 `firmware/handheld` 中编译（下面以 rev4 为例）：

```sh
# 默认：上下选择；左右键或 L/R 翻页，也可选择 Previous/Next page 条目
./build_fusion_pixel_full.sh --release --features=rev4

# 滚动：上下选择，到当前批次边界时自动加载前／后一批
./build_fusion_pixel_full.sh --release --features=rev4,rom-list-scroll
```

滚动模式不显示翻页条目，列表首尾停止；跨批次会短暂显示 Loading。
两种模式都支持 A 打开、B 返回上级目录及恢复上次选中的文件。

两种模式共用有界目录扫描：每批最多 32 个文件／目录，扫描堆最多保留
33 个候选；UI 最多 35 行（含上级目录及翻页条目）。加载前清空旧模型，
不缓存或追加历史批次，因此列表内存不会随目录条目总数增长。
文件名保持完整用于打开文件，显示宽度限制为 270px；全量中文字库覆盖文本
仍只在编译时使用。此约束避免全目录加载导致的 OOM；设备整体剩余堆、
渲染缓存和长期运行仍需实机压力测试确认。

回归及内存测试见 [ROM selector host checks](firmware/handheld/tests/rom-selector-host/README.md)。
