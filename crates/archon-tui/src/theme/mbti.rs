//! 16 MBTI-typed theme constructors.
//!
//! Relocated from `src/theme.rs` (lines 127-432) per REM-2i. Data-only module:
//! each `*_theme()` returns a `Theme` struct literal — no shared state, no
//! closures, no private helpers.

use ratatui::style::Color;

use super::Theme;

/// Return the default INTJ theme.
pub fn intj_theme() -> Theme {
    Theme {
        bg: Color::Black,
        fg: Color::Rgb(200, 200, 210),
        accent: Color::Rgb(0, 180, 180),
        accent_secondary: Color::Rgb(130, 80, 200),
        header: Color::Rgb(0, 220, 220),
        muted: Color::DarkGray,
        error: Color::Rgb(200, 60, 60),
        success: Color::Rgb(0, 200, 120),
        warning: Color::Rgb(200, 180, 0),
        border: Color::Rgb(60, 60, 120),
        border_active: Color::Rgb(80, 120, 220),
        thinking_dot: Color::Rgb(0, 180, 180),
        thinking_dot_bright: Color::Rgb(0, 255, 255),
    }
}

// ---------------------------------------------------------------------------
// MBTI themes
// ---------------------------------------------------------------------------

/// INTP — The Logician. Cold blue-grey analytical precision.
pub fn intp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 12, 20),
        fg: Color::Rgb(180, 185, 205),
        accent: Color::Rgb(80, 130, 200),
        accent_secondary: Color::Rgb(100, 100, 160),
        header: Color::Rgb(130, 175, 245),
        muted: Color::Rgb(70, 80, 100),
        error: Color::Rgb(200, 60, 60),
        success: Color::Rgb(60, 200, 120),
        warning: Color::Rgb(200, 180, 40),
        border: Color::Rgb(35, 45, 75),
        border_active: Color::Rgb(80, 130, 200),
        thinking_dot: Color::Rgb(80, 130, 200),
        thinking_dot_bright: Color::Rgb(160, 205, 255),
    }
}

/// ENTJ — The Commander. Imperial deep blue and gold.
pub fn entj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 8, 18),
        fg: Color::Rgb(225, 215, 185),
        accent: Color::Rgb(200, 160, 20),
        accent_secondary: Color::Rgb(40, 80, 180),
        header: Color::Rgb(245, 200, 40),
        muted: Color::Rgb(100, 90, 60),
        error: Color::Rgb(220, 60, 40),
        success: Color::Rgb(60, 200, 100),
        warning: Color::Rgb(240, 160, 20),
        border: Color::Rgb(55, 45, 15),
        border_active: Color::Rgb(200, 160, 20),
        thinking_dot: Color::Rgb(200, 160, 20),
        thinking_dot_bright: Color::Rgb(255, 220, 80),
    }
}

/// ENTP — The Debater. Electric cyan-green, innovative spark.
pub fn entp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(5, 15, 15),
        fg: Color::Rgb(180, 240, 205),
        accent: Color::Rgb(0, 220, 150),
        accent_secondary: Color::Rgb(80, 200, 255),
        header: Color::Rgb(100, 255, 185),
        muted: Color::Rgb(60, 100, 80),
        error: Color::Rgb(220, 80, 60),
        success: Color::Rgb(0, 220, 150),
        warning: Color::Rgb(200, 200, 40),
        border: Color::Rgb(15, 70, 55),
        border_active: Color::Rgb(0, 200, 150),
        thinking_dot: Color::Rgb(0, 220, 150),
        thinking_dot_bright: Color::Rgb(100, 255, 205),
    }
}

/// INFJ — The Advocate. Deep violet and indigo, mystical.
pub fn infj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(10, 5, 20),
        fg: Color::Rgb(210, 190, 235),
        accent: Color::Rgb(140, 80, 220),
        accent_secondary: Color::Rgb(80, 120, 200),
        header: Color::Rgb(185, 125, 255),
        muted: Color::Rgb(80, 60, 100),
        error: Color::Rgb(220, 60, 80),
        success: Color::Rgb(60, 200, 130),
        warning: Color::Rgb(200, 160, 40),
        border: Color::Rgb(50, 28, 80),
        border_active: Color::Rgb(140, 80, 220),
        thinking_dot: Color::Rgb(140, 80, 220),
        thinking_dot_bright: Color::Rgb(205, 155, 255),
    }
}

/// INFP — The Mediator. Soft rose and warm lavender, dreamy.
pub fn infp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(15, 8, 15),
        fg: Color::Rgb(235, 215, 225),
        accent: Color::Rgb(200, 100, 160),
        accent_secondary: Color::Rgb(160, 100, 200),
        header: Color::Rgb(245, 145, 195),
        muted: Color::Rgb(100, 70, 90),
        error: Color::Rgb(220, 60, 80),
        success: Color::Rgb(80, 200, 130),
        warning: Color::Rgb(220, 170, 60),
        border: Color::Rgb(80, 38, 58),
        border_active: Color::Rgb(200, 100, 160),
        thinking_dot: Color::Rgb(200, 100, 160),
        thinking_dot_bright: Color::Rgb(255, 165, 205),
    }
}

/// ENFJ — The Protagonist. Warm amber and gold, charismatic warmth.
pub fn enfj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 5),
        fg: Color::Rgb(245, 225, 195),
        accent: Color::Rgb(220, 140, 20),
        accent_secondary: Color::Rgb(180, 80, 40),
        header: Color::Rgb(255, 185, 45),
        muted: Color::Rgb(120, 90, 60),
        error: Color::Rgb(220, 60, 40),
        success: Color::Rgb(60, 200, 100),
        warning: Color::Rgb(240, 180, 20),
        border: Color::Rgb(95, 55, 18),
        border_active: Color::Rgb(220, 140, 20),
        thinking_dot: Color::Rgb(220, 140, 20),
        thinking_dot_bright: Color::Rgb(255, 205, 85),
    }
}

/// ENFP — The Campaigner. Bright coral and sunshine, boundless enthusiasm.
pub fn enfp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 5),
        fg: Color::Rgb(252, 235, 205),
        accent: Color::Rgb(255, 120, 60),
        accent_secondary: Color::Rgb(255, 200, 40),
        header: Color::Rgb(255, 165, 85),
        muted: Color::Rgb(130, 100, 70),
        error: Color::Rgb(220, 50, 50),
        success: Color::Rgb(60, 210, 100),
        warning: Color::Rgb(255, 200, 40),
        border: Color::Rgb(115, 58, 18),
        border_active: Color::Rgb(255, 120, 60),
        thinking_dot: Color::Rgb(255, 120, 60),
        thinking_dot_bright: Color::Rgb(255, 205, 125),
    }
}

/// ISTJ — The Inspector. Military green and navy, steadfast duty.
pub fn istj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(5, 10, 8),
        fg: Color::Rgb(200, 210, 195),
        accent: Color::Rgb(80, 140, 80),
        accent_secondary: Color::Rgb(40, 80, 120),
        header: Color::Rgb(105, 185, 105),
        muted: Color::Rgb(70, 90, 70),
        error: Color::Rgb(200, 60, 60),
        success: Color::Rgb(80, 200, 80),
        warning: Color::Rgb(200, 180, 40),
        border: Color::Rgb(28, 55, 28),
        border_active: Color::Rgb(80, 140, 80),
        thinking_dot: Color::Rgb(80, 140, 80),
        thinking_dot_bright: Color::Rgb(140, 225, 140),
    }
}

/// ISFJ — The Defender. Warm brown and parchment, nurturing earth.
pub fn isfj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(18, 12, 8),
        fg: Color::Rgb(235, 215, 195),
        accent: Color::Rgb(180, 120, 60),
        accent_secondary: Color::Rgb(160, 140, 80),
        header: Color::Rgb(225, 165, 85),
        muted: Color::Rgb(105, 80, 60),
        error: Color::Rgb(210, 60, 50),
        success: Color::Rgb(80, 190, 80),
        warning: Color::Rgb(220, 175, 40),
        border: Color::Rgb(78, 52, 28),
        border_active: Color::Rgb(180, 120, 60),
        thinking_dot: Color::Rgb(180, 120, 60),
        thinking_dot_bright: Color::Rgb(242, 185, 105),
    }
}

/// ESTJ — The Executive. Bold navy and dark red, commanding authority.
pub fn estj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 8, 15),
        fg: Color::Rgb(220, 218, 212),
        accent: Color::Rgb(40, 80, 180),
        accent_secondary: Color::Rgb(180, 40, 40),
        header: Color::Rgb(85, 145, 225),
        muted: Color::Rgb(80, 80, 95),
        error: Color::Rgb(200, 50, 50),
        success: Color::Rgb(50, 200, 100),
        warning: Color::Rgb(210, 170, 30),
        border: Color::Rgb(28, 38, 78),
        border_active: Color::Rgb(40, 80, 180),
        thinking_dot: Color::Rgb(40, 80, 180),
        thinking_dot_bright: Color::Rgb(105, 165, 255),
    }
}

/// ESFJ — The Consul. Warm pink and friendly coral, community warmth.
pub fn esfj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 12),
        fg: Color::Rgb(245, 225, 225),
        accent: Color::Rgb(220, 80, 120),
        accent_secondary: Color::Rgb(240, 140, 80),
        header: Color::Rgb(255, 125, 155),
        muted: Color::Rgb(120, 80, 95),
        error: Color::Rgb(220, 50, 70),
        success: Color::Rgb(60, 200, 110),
        warning: Color::Rgb(240, 175, 40),
        border: Color::Rgb(98, 38, 58),
        border_active: Color::Rgb(220, 80, 120),
        thinking_dot: Color::Rgb(220, 80, 120),
        thinking_dot_bright: Color::Rgb(255, 165, 195),
    }
}

/// ISTP — The Virtuoso. Steel grey and gunmetal, mechanical precision.
pub fn istp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 10, 10),
        fg: Color::Rgb(200, 208, 200),
        accent: Color::Rgb(140, 160, 160),
        accent_secondary: Color::Rgb(100, 120, 100),
        header: Color::Rgb(185, 205, 205),
        muted: Color::Rgb(80, 92, 85),
        error: Color::Rgb(200, 70, 60),
        success: Color::Rgb(80, 195, 100),
        warning: Color::Rgb(200, 185, 50),
        border: Color::Rgb(48, 58, 55),
        border_active: Color::Rgb(140, 160, 160),
        thinking_dot: Color::Rgb(140, 160, 160),
        thinking_dot_bright: Color::Rgb(212, 235, 235),
    }
}

/// ISFP — The Adventurer. Warm sand and earth tones, quiet artistry.
pub fn isfp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(15, 12, 8),
        fg: Color::Rgb(228, 218, 202),
        accent: Color::Rgb(180, 150, 80),
        accent_secondary: Color::Rgb(140, 100, 80),
        header: Color::Rgb(225, 195, 105),
        muted: Color::Rgb(112, 96, 75),
        error: Color::Rgb(210, 65, 55),
        success: Color::Rgb(80, 195, 95),
        warning: Color::Rgb(215, 185, 50),
        border: Color::Rgb(78, 62, 38),
        border_active: Color::Rgb(180, 150, 80),
        thinking_dot: Color::Rgb(180, 150, 80),
        thinking_dot_bright: Color::Rgb(242, 212, 132),
    }
}

/// ESTP — The Entrepreneur. Bold red and energetic orange, live action.
pub fn estp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 5, 5),
        fg: Color::Rgb(242, 222, 202),
        accent: Color::Rgb(220, 50, 30),
        accent_secondary: Color::Rgb(240, 140, 0),
        header: Color::Rgb(255, 85, 55),
        muted: Color::Rgb(120, 70, 60),
        error: Color::Rgb(255, 40, 40),
        success: Color::Rgb(60, 210, 80),
        warning: Color::Rgb(240, 160, 20),
        border: Color::Rgb(98, 28, 18),
        border_active: Color::Rgb(220, 50, 30),
        thinking_dot: Color::Rgb(220, 50, 30),
        thinking_dot_bright: Color::Rgb(255, 125, 85),
    }
}

/// ESFP — The Entertainer. Bright yellow and hot pink, pure fun.
pub fn esfp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 15),
        fg: Color::Rgb(252, 242, 212),
        accent: Color::Rgb(240, 180, 0),
        accent_secondary: Color::Rgb(220, 60, 140),
        header: Color::Rgb(255, 225, 45),
        muted: Color::Rgb(122, 102, 82),
        error: Color::Rgb(220, 50, 80),
        success: Color::Rgb(60, 210, 100),
        warning: Color::Rgb(240, 200, 20),
        border: Color::Rgb(98, 68, 18),
        border_active: Color::Rgb(240, 180, 0),
        thinking_dot: Color::Rgb(240, 180, 0),
        thinking_dot_bright: Color::Rgb(255, 242, 85),
    }
}
