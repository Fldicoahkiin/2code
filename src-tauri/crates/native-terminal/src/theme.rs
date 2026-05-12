/// Terminal color theme — mirrors xterm.js ITheme structure.
#[derive(Clone)]
pub struct TerminalTheme {
	pub background: (u8, u8, u8),
	pub foreground: (u8, u8, u8),
	pub cursor: (u8, u8, u8),
	/// 16 ANSI colors: [black, red, green, yellow, blue, magenta, cyan, white,
	/// bright_black, bright_red, bright_green, bright_yellow, bright_blue,
	/// bright_magenta, bright_cyan, bright_white]
	pub ansi: [(u8, u8, u8); 16],
}

impl Default for TerminalTheme {
	fn default() -> Self {
		Self {
			background: (22, 22, 22),    // #161616 (github-dark)
			foreground: (191, 212, 225), // #BFD4E1
			cursor: (240, 243, 189),     // #f0f3bd
			ansi: [
				(53, 53, 53),    // black     #353535
				(217, 115, 151), // red       #d97397
				(206, 227, 151), // green     #CEE397
				(233, 202, 92),  // yellow    #E9CA5C
				(99, 176, 198),  // blue      #63B0C6
				(233, 174, 186), // magenta   #E9AEBA
				(112, 193, 179), // cyan      #70C1B3
				(191, 212, 225), // white     #BFD4E1
				(114, 144, 152), // br_black  #729098
				(255, 173, 173), // br_red    #ffadad
				(202, 255, 191), // br_green  #caffbf
				(240, 243, 189), // br_yellow #f0f3bd
				(155, 246, 255), // br_blue   #9bf6ff
				(255, 198, 255), // br_magenta #ffc6ff
				(168, 218, 220), // br_cyan   #a8dadc
				(255, 255, 255), // br_white  #ffffff
			],
		}
	}
}

impl TerminalTheme {
	/// Create theme from hex color strings (as sent from frontend).
	/// Each string should be "#RRGGBB" format.
	pub fn from_hex(
		background: &str,
		foreground: &str,
		cursor: &str,
		ansi_colors: &[String; 16],
	) -> Self {
		Self {
			background: parse_hex(background),
			foreground: parse_hex(foreground),
			cursor: parse_hex(cursor),
			ansi: [
				parse_hex(&ansi_colors[0]),
				parse_hex(&ansi_colors[1]),
				parse_hex(&ansi_colors[2]),
				parse_hex(&ansi_colors[3]),
				parse_hex(&ansi_colors[4]),
				parse_hex(&ansi_colors[5]),
				parse_hex(&ansi_colors[6]),
				parse_hex(&ansi_colors[7]),
				parse_hex(&ansi_colors[8]),
				parse_hex(&ansi_colors[9]),
				parse_hex(&ansi_colors[10]),
				parse_hex(&ansi_colors[11]),
				parse_hex(&ansi_colors[12]),
				parse_hex(&ansi_colors[13]),
				parse_hex(&ansi_colors[14]),
				parse_hex(&ansi_colors[15]),
			],
		}
	}

	/// Look up named ANSI color index (0–15).
	pub fn named_color(&self, idx: usize) -> (u8, u8, u8) {
		if idx < 16 {
			self.ansi[idx]
		} else {
			self.foreground
		}
	}
}

fn parse_hex(hex: &str) -> (u8, u8, u8) {
	let hex = hex.trim_start_matches('#');
	if hex.len() >= 6 {
		let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(220);
		let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(220);
		let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(220);
		(r, g, b)
	} else {
		(220, 220, 220)
	}
}
