#!/usr/bin/env bash
# Machine settings for the benchmarks (SPEC section 14, "Mesure").
#   machine.sh apply [single|parallel]
#       fixed frequency (governor `performance`, turbo off, minimum = maximum),
#       CPU counters allowed for perf, and isolated CPUs:
#       single    the physical cores are split in two halves: every other
#                 process is confined to the first half (all their hardware
#                 threads), the benchmarks get the second half and are pinned
#                 to the first thread of its last core (its sibling stays idle)
#       parallel  the benchmarks get the first hardware thread of every
#                 physical core; every other process is confined to the
#                 sibling threads (without SMT: no confinement)
#       The state before `apply` is saved first.
#   machine.sh run ...   runs `bench/run.py ...` (or `--script f.py ...`) in its
#                        own slice on the benchmark CPUs
#   machine.sh restore   puts back the saved state, every process on every CPU
#   machine.sh status    prints the current state
# Everything is --runtime or sysfs: a reboot also restores it.
# Needs sudo without a password.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
saved="$here/build/machine.saved"
envf="$here/build/machine.env"
cpus=/sys/devices/system/cpu
turbo=$cpus/intel_pstate/no_turbo
units="user.slice system.slice init.scope"

status() {
    echo "governor: $(cat $cpus/cpu*/cpufreq/scaling_governor | sort -u | tr '\n' ' ')"
    echo "no_turbo: $(cat $turbo 2>/dev/null || echo n/a)"
    echo "freq min/max: $(cat $cpus/cpu0/cpufreq/scaling_min_freq)/$(cat $cpus/cpu0/cpufreq/scaling_max_freq) kHz"
    echo "perf_event_paranoid: $(cat /proc/sys/kernel/perf_event_paranoid)"
    # the CPUs the cgroup really has: systemd's property alone can look
    # unrestricted while the cgroup still is (see restore)
    for u in $units; do
        echo "$u AllowedCPUs: [$(systemctl show -p AllowedCPUs --value $u)] effective: $(cat /sys/fs/cgroup/$u/cpuset.cpus.effective 2>/dev/null || echo n/a)"
    done
    [ -f "$envf" ] && cat "$envf"
    return 0
}

# Prints "OTHERS BENCH PIN" (comma-separated CPU lists) for a mode.
plan() {
    python3 -I - "$1" <<'EOF'
import subprocess, sys
mode = sys.argv[1]
rows = [l.split(",") for l in subprocess.run(["lscpu", "-p=CPU,CORE"], capture_output=True, text=True).stdout.splitlines()
        if l and not l.startswith("#")]
cores = {}
for cpu, core in rows:
    cores.setdefault(int(core), []).append(int(cpu))
order = sorted(cores)
threads = [sorted(cores[c]) for c in order]
if mode == "single":
    half = len(threads) // 2
    others = [c for t in threads[:half] for c in t]
    bench = [c for t in threads[half:] for c in t]
    pin = threads[-1][0]
elif mode == "parallel":
    bench = [t[0] for t in threads]
    others = [c for t in threads for c in t[1:]]
    pin = threads[-1][0]
else:
    sys.exit("unknown mode " + mode)
j = lambda xs: ",".join(map(str, xs))
print(j(others) or "-", j(bench), pin)
EOF
}

case "${1:-status}" in
apply)
    mode="${2:-single}"
    mkdir -p "$here/build"
    if [ ! -f "$saved" ]; then
        {
            echo "governor=$(cat $cpus/cpu0/cpufreq/scaling_governor)"
            echo "no_turbo=$(cat $turbo 2>/dev/null || echo)"
            echo "min_freq=$(cat $cpus/cpu0/cpufreq/scaling_min_freq)"
            echo "paranoid=$(cat /proc/sys/kernel/perf_event_paranoid)"
        } > "$saved"
    fi
    read -r others bench pin < <(plan "$mode")
    sudo -n cpupower frequency-set -g performance > /dev/null
    [ -f $turbo ] && echo 1 | sudo -n tee $turbo > /dev/null
    # minimum = maximum (the maximum is the base frequency once the turbo is off)
    sudo -n cpupower frequency-set -d "$(cat $cpus/cpu0/cpufreq/scaling_max_freq)kHz" > /dev/null
    sudo -n sysctl -q kernel.perf_event_paranoid=1
    # Free memory in huge pages for every program: when the page cache fills
    # the memory, only a program that asks for huge pages (madvise) makes the
    # kernel compact memory for it; Rust and Zig then got pages of 4 KiB and
    # Isocron huge pages (DECISIONS D84). The cache fills again by itself.
    sync
    echo 1 | sudo -n tee /proc/sys/vm/drop_caches > /dev/null
    sudo -n sysctl -q vm.compact_memory=1
    for u in $units; do
        if [ "$others" = "-" ]; then
            sudo -n systemctl set-property --runtime "$u" AllowedCPUs="$(cat $cpus/online)"
        else
            sudo -n systemctl set-property --runtime "$u" AllowedCPUs="$others"
        fi
    done
    printf 'MODE=%s\nBENCH_CPUS=%s\nPIN_CPU=%s\n' "$mode" "$bench" "$pin" > "$envf"
    status
    ;;
run)
    shift
    [ -f "$envf" ] || { echo "run 'machine.sh apply' first" >&2; exit 1; }
    # shellcheck disable=SC1090
    . "$envf"
    script="$here/run.py"
    if [ "${1:-}" = "--script" ]; then
        script="$2"
        shift 2
    fi
    exec sudo -n systemd-run --quiet --scope --slice=isc-bench -p AllowedCPUs="$BENCH_CPUS" \
        --uid="$(id -u)" --gid="$(id -g)" -E HOME="$HOME" -E PATH="$PATH" \
        -E ISC_BENCH_CPU="$PIN_CPU" -E ISC_BENCH_CPUS="$BENCH_CPUS" \
        python3 -I "$script" "$@"
    ;;
restore)
    # Every CPU back to every process, even when nothing is saved. Given
    # explicitly: `AllowedCPUs=` (empty) removes systemd's property but
    # leaves the cgroup's cpuset as it was, so that the processes stayed
    # confined after every restore before this fix (DECISIONS D80).
    for u in $units; do sudo -n systemctl set-property --runtime "$u" AllowedCPUs="$(cat $cpus/online)"; done
    if [ -f "$saved" ]; then
        # shellcheck disable=SC1090
        . "$saved"
        sudo -n cpupower frequency-set -g "$governor" > /dev/null
        [ -n "$no_turbo" ] && echo "$no_turbo" | sudo -n tee $turbo > /dev/null
        sudo -n cpupower frequency-set -d "${min_freq}kHz" > /dev/null
        sudo -n sysctl -q kernel.perf_event_paranoid="$paranoid"
        mv "$saved" "$saved.restored"
    else
        echo "nothing saved in $saved: only the CPUs were given back" >&2
    fi
    rm -f "$envf"
    status
    ;;
status) status ;;
*) echo "usage: $0 apply [single|parallel] | run [--script f.py] ... | restore | status" >&2; exit 2 ;;
esac
