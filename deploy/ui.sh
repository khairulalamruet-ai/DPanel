# shellcheck shell=bash
# The installer's look (sourced by install.sh; get.sh carries a copy, put in by the release).
#
#   ui_init                 colours and fd 3 (the terminal); call first
#   ui_banner "subtitle"    the Built Panel logo, drawn line by line, and a typed subtitle
#   ui_log_to FILE          from now on the commands' own output goes to FILE, the terminal shows
#                           only the steps (nothing changes in plain mode)
#   ui_step "label"         ends the step before (✔ and its time) and starts this one: a spinner,
#                           the step number and an overall progress bar that keeps moving
#   ui_warn "text"          a note under the current step
#   ui_fail "text"          ✖ on the current step, the reason and the end of the log
#   ui_done                 ends the last step and fills the bar
#   ui_box < lines          a framed summary
#   ui_cleanup              stops the spinner and shows the cursor again (call from an EXIT trap)
#
# Plain lines ("==> step", no colours, no animation) when the output is not a terminal, TERM is
# dumb, NO_COLOR is set or BPINSTALL_PLAIN=1: logs and CI stay readable.

UI_FANCY=0; UI_I=0; UI_TOTAL=${UI_TOTAL:-10}; UI_LABEL=""; UI_PID=""; UI_LOG=""
UI_T0=$SECONDS; UI_STEP_T0=$SECONDS; UI_COLS=80
UI_FLAG=""; UI_R=""; UI_B=""; UI_DIM=""; UI_OK=""; UI_BAD=""; UI_ACC=""; UI_MUT=""; UI_WARN=""

ui_init() {
    exec 3>&1 4>&2
    if [ -t 1 ] && [ "${TERM:-dumb}" != dumb ] && [ -z "${NO_COLOR:-}" ] && [ "${BPINSTALL_PLAIN:-0}" != 1 ]; then
        UI_FANCY=1
        UI_FLAG=$(mktemp "${TMPDIR:-/tmp}/builtpanel-ui.XXXXXX")
        rm -f "$UI_FLAG"
        UI_R=$'\e[0m'; UI_B=$'\e[1m'; UI_DIM=$'\e[2m'
        UI_OK=$'\e[38;5;42m'; UI_BAD=$'\e[38;5;203m'; UI_ACC=$'\e[38;5;45m'
        UI_MUT=$'\e[38;5;244m'; UI_WARN=$'\e[38;5;214m'
    fi
    UI_COLS=$(tput cols 2> /dev/null || echo 80)
    [[ "$UI_COLS" =~ ^[0-9]+$ ]] || UI_COLS=80
}

_ui_logo() { # the lines of the logo, one or two rows of letters depending on the width
    local one=(
        '██████╗ ██╗   ██╗██╗██╗     ████████╗  ██████╗  █████╗ ███╗   ██╗███████╗██╗'
        '██╔══██╗██║   ██║██║██║     ╚══██╔══╝  ██╔══██╗██╔══██╗████╗  ██║██╔════╝██║'
        '██████╔╝██║   ██║██║██║        ██║     ██████╔╝███████║██╔██╗ ██║█████╗  ██║'
        '██╔══██╗██║   ██║██║██║        ██║     ██╔═══╝ ██╔══██║██║╚██╗██║██╔══╝  ██║'
        '██████╔╝╚██████╔╝██║███████╗   ██║     ██║     ██║  ██║██║ ╚████║███████╗███████╗'
        '╚═════╝  ╚═════╝ ╚═╝╚══════╝   ╚═╝     ╚═╝     ╚═╝  ╚═╝╚═╝  ╚═══╝╚══════╝╚══════╝'
    )
    local two=(
        '██████╗ ██╗   ██╗██╗██╗     ████████╗'
        '██╔══██╗██║   ██║██║██║     ╚══██╔══╝'
        '██████╔╝██║   ██║██║██║        ██║'
        '██╔══██╗██║   ██║██║██║        ██║'
        '██████╔╝╚██████╔╝██║███████╗   ██║'
        '╚═════╝  ╚═════╝ ╚═╝╚══════╝   ╚═╝'
        '██████╗  █████╗ ███╗   ██╗███████╗██╗'
        '██╔══██╗██╔══██╗████╗  ██║██╔════╝██║'
        '██████╔╝███████║██╔██╗ ██║█████╗  ██║'
        '██╔═══╝ ██╔══██║██║╚██╗██║██╔══╝  ██║'
        '██║     ██║  ██║██║ ╚████║███████╗███████╗'
        '╚═╝     ╚═╝  ╚═╝╚═╝  ╚═══╝╚══════╝╚══════╝'
    )
    if [ "$UI_COLS" -ge 86 ]; then printf '%s\n' "${one[@]}"; else printf '%s\n' "${two[@]}"; fi
}

ui_banner() { # subtitle
    local subtitle="${1:-}" line i=0 colours=(51 45 39 33 63 99 51 45 39 33 63 99)
    if [ "$UI_FANCY" -ne 1 ]; then
        printf '\nBuilt Panel%s\n\n' "${subtitle:+ - $subtitle}" >&3
        return 0
    fi
    printf '\e[?25l\n' >&3
    while IFS= read -r line; do
        printf '  \e[1;38;5;%sm%s%s\n' "${colours[i]}" "$line" "$UI_R" >&3
        i=$((i + 1))
        sleep 0.035
    done < <(_ui_logo)
    if [ -n "$subtitle" ]; then
        printf '\n  %s' "$UI_MUT" >&3
        for ((i = 0; i < ${#subtitle}; i++)); do
            printf '%s' "${subtitle:i:1}" >&3
            sleep 0.008
        done
        printf '%s\n' "$UI_R" >&3
    fi
    printf '  %s%s%s\n\n' "$UI_ACC" "$(printf '─%.0s' $(seq 1 $((UI_COLS < 86 ? 42 : 81))))" "$UI_R" >&3
}

ui_log_to() { # file
    [ "$UI_FANCY" -eq 1 ] || return 0
    UI_LOG="$1"
    install -d -m 0755 "$(dirname "$UI_LOG")" 2> /dev/null || true
    : >> "$UI_LOG"
    chmod 0600 "$UI_LOG" 2> /dev/null || true
    exec >> "$UI_LOG" 2>&1
    printf '  %sfull log: %s%s\n\n' "$UI_DIM" "$UI_LOG" "$UI_R" >&3
}

_ui_clock() { # seconds -> m:ss
    printf '%d:%02d' $(($1 / 60)) $(($1 % 60))
}

_ui_bar() { # percent, pulse (0/1) -> the bar
    local width=26 pct=$1 full rest out=""
    full=$((pct * width / 100))
    rest=$((width - full))
    [ "$full" -gt 0 ] && out+="$(printf '█%.0s' $(seq 1 "$full"))"
    if [ "$rest" -gt 0 ]; then
        if [ "$2" -eq 1 ]; then out+="▓"; else out+="▒"; fi
        [ "$rest" -gt 1 ] && out+="${UI_MUT}$(printf '░%.0s' $(seq 1 $((rest - 1))))"
    fi
    printf '%s%s%s' "$UI_ACC" "$out" "$UI_R"
}

_ui_spin() { # runs in the background until killed
    local frames=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏) k=0 base step_pct pct label width
    base=$(((UI_I - 1) * 100 / UI_TOTAL))
    step_pct=$((100 / UI_TOTAL))
    width=$((UI_COLS - 62))
    [ "$width" -ge 16 ] || width=16
    label="$UI_LABEL"
    [ "${#label}" -le "$width" ] || label="${label:0:$((width - 1))}…"
    while [ -e "$UI_FLAG" ]; do
        # Creeps forward within the step (never past it) so the bar is always alive.
        pct=$((base + (k / 8 < step_pct ? k / 8 : step_pct - 1)))
        [ "$pct" -le 99 ] || pct=99
        printf '\r\e[K  %s%s%s %s%2d/%-2d%s %-*s %s %s%3d%%  %s%s' \
            "$UI_ACC" "${frames[k % 10]}" "$UI_R" "$UI_MUT" "$UI_I" "$UI_TOTAL" "$UI_R" \
            "$width" "$label" "$(_ui_bar "$pct" $((k / 3 % 2)))" "$UI_B" "$pct" \
            "$(_ui_clock $((SECONDS - UI_T0)))" "$UI_R" >&3
        k=$((k + 1))
        sleep 0.09
    done
}

# The spinner runs in the background and stops by itself when its flag file is removed (it is
# not killed: a killed job is reported by bash, and stopping it by signal raced with this
# shell's own cleanup). It is a copy of this shell, so it would run this shell's EXIT trap when
# it ends: a trap that must run once (removing the install's work folder) has to check
# `ui_is_main` first.
_ui_start_spinner() {
    : > "$UI_FLAG"
    _ui_spin &
    UI_PID=$!
}

_ui_stop_spinner() {
    if [ -n "$UI_PID" ]; then
        rm -f "$UI_FLAG"
        wait "$UI_PID" 2> /dev/null || true
        UI_PID=""
    fi
}

UI_MAIN_PID=$$
ui_is_main() { # true in the script itself, false in the spinner (a copy of it)
    [ "${BASHPID:-$$}" = "$UI_MAIN_PID" ]
}

_ui_end_step() {
    [ -n "$UI_LABEL" ] || return 0
    _ui_stop_spinner
    if [ "$UI_FANCY" -eq 1 ]; then
        printf '\r\e[K  %s✔%s  %s %s(%ss)%s\n' "$UI_OK" "$UI_R" "$UI_LABEL" "$UI_DIM" \
            $((SECONDS - UI_STEP_T0)) "$UI_R" >&3
    fi
    UI_LABEL=""
}

ui_step() { # label
    _ui_end_step
    UI_I=$((UI_I + 1))
    [ "$UI_I" -le "$UI_TOTAL" ] || UI_TOTAL=$UI_I
    UI_LABEL="$1"
    UI_STEP_T0=$SECONDS
    echo "==> $1"
    if [ "$UI_FANCY" -eq 1 ]; then _ui_start_spinner; fi
}

ui_warn() { # text
    echo "warning: $1"
    if [ "$UI_FANCY" -eq 1 ]; then
        _ui_stop_spinner
        printf '\r\e[K     %s! %s%s\n' "$UI_WARN" "$1" "$UI_R" >&3
        if [ -n "$UI_LABEL" ]; then _ui_start_spinner; fi
    fi
}

ui_fail() { # text
    _ui_stop_spinner
    if [ "$UI_FANCY" -eq 1 ]; then
        printf '\r\e[K  %s✖%s  %s\n\n  %s%s%s%s\n' "$UI_BAD" "$UI_R" "${UI_LABEL:-starting}" \
            "$UI_BAD" "$UI_B" "$1" "$UI_R" >&3
        if [ -n "$UI_LOG" ] && [ -s "$UI_LOG" ]; then
            printf '\n  %sthe end of %s:%s\n' "$UI_MUT" "$UI_LOG" "$UI_R" >&3
            tail -n 15 "$UI_LOG" | sed "s/^/  ${UI_DIM}│${UI_R} /" >&3
        fi
        printf '\e[?25h\n' >&3
    else
        echo "error: $1" >&4
    fi
    UI_LABEL=""
}

ui_done() {
    _ui_end_step
    if [ "$UI_FANCY" -eq 1 ]; then
        printf '\n  %s✔%s  %sall done%s %s %s100%%%s  %s\n\n' "$UI_OK" "$UI_R" "$UI_B" "$UI_R" \
            "$(_ui_bar 100 0)" "$UI_B" "$UI_R" "$(_ui_clock $((SECONDS - UI_T0)))" >&3
        printf '\e[?25h' >&3
    fi
}

ui_box() { # the lines come on stdin
    local lines=() line width=0 plain
    while IFS= read -r line; do lines+=("$line"); done
    if [ "$UI_FANCY" -ne 1 ]; then
        printf '%s\n' "${lines[@]}" >&3
        return 0
    fi
    for line in "${lines[@]}"; do
        plain=$(printf '%s' "$line" | sed $'s/\e\\[[0-9;]*m//g')
        [ "${#plain}" -le "$width" ] || width=${#plain}
    done
    printf '  %s╭%s╮%s\n' "$UI_ACC" "$(printf '─%.0s' $(seq 1 $((width + 2))))" "$UI_R" >&3
    for line in "${lines[@]}"; do
        plain=$(printf '%s' "$line" | sed $'s/\e\\[[0-9;]*m//g')
        printf '  %s│%s %s%*s %s│%s\n' "$UI_ACC" "$UI_R" "$line" $((width - ${#plain})) '' \
            "$UI_ACC" "$UI_R" >&3
    done
    printf '  %s╰%s╯%s\n' "$UI_ACC" "$(printf '─%.0s' $(seq 1 $((width + 2))))" "$UI_R" >&3
}

ui_cleanup() {
    _ui_stop_spinner
    [ -z "$UI_FLAG" ] || rm -f "$UI_FLAG"
    [ "$UI_FANCY" -ne 1 ] || printf '\e[?25h' >&3 2> /dev/null || true
}
