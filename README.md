# gpg-bridge
A bridge connects openssh-portable and GnuPG on Windows.

A fork of BusyJay's gpg-bridge at : https://github.com/BusyJay/gpg-bridge

This version simplifies the codebase by removing the support for `--ssh`.

## Usage

1. Install it on your System.

    ```
    cargo install -f --locked --git https://github.com/cqjjjzr/gpg-bridge
    ```

    It's recommanded to build the binary yourself. In case you don't want to setup Rust environment,
    you can also download a prebuilt binary from [GitHub Releases][2], or the artifact of any commit's
    build workflow ([how to download artifacts][1]).

2. Make sure you have setup gpg agent forward following [the guide](https://wiki.gnupg.org/AgentForwarding).

3. Directly using socket provided by GnuPG won't work on Windows, so change local socket to a TCP port instead.

    ```
    RemoteForward <socket_on_remote_box>  127.0.0.1:4321
    ```

    You are free to use any port that has not been taken, `4321` is just an example.

4. Build a bridge between TCP port and GnuPG extra socket.

    ```
    ~/.cargo/bin/gpg-bridge --extra 127.0.0.1:4321
    ```

    If you have customized extra socket localtion, you set the path using `--extra-socket`.

    Only loopback addresses are accepted. Note that any local process, including other users' on a
    shared machine, can reach your agent's extra socket through the bridge.

Now you are all set, requests to gpg agent on remote should be able to forward to your local.

## Running in background

`gpg-bridgew.exe` is the same program without a console window. Use it with `--log-file` to keep logs
(set `RUST_LOG=debug` for more detail). To start it at logon with Task Scheduler, run in PowerShell:

```powershell
$action = New-ScheduledTaskAction -Execute "$env:USERPROFILE\.cargo\bin\gpg-bridgew.exe" `
    -Argument "--extra 127.0.0.1:4321 --log-file `"$env:LOCALAPPDATA\gpg-bridge.log`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User ([Security.Principal.WindowsIdentity]::GetCurrent().Name)
# Defaults would stop it after 3 days or on battery.
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit 0 -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask gpg-bridge -Action $action -Trigger $trigger -Settings $settings
Start-ScheduledTask gpg-bridge
```

Task Scheduler then shows whether it is running, its exit code, and can stop it. Remove it with
`Unregister-ScheduledTask gpg-bridge`.

A Windows service is not a good fit: services run in session 0, apart from the per-user gpg-agent and
its pinentry.

## Why invent the wheel

There are several gotchas if not using bridge to forward gpg agent on Windows. See PowerShell/Win32-OpenSSH#1564.

1. Specifying remote forward local socket path in openssh-portable can be tricky (for now).

    Path like `C:/xxx`, `~/xxx` and `%userprofile%/xxx` will not work. You have to use form like
    `/absolute/path/to/local/socket` and execute ssh on the same driver path. See
    https://docs.microsoft.com/en-us/dotnet/standard/io/file-path-formats.

2. Even path is correctly specified and accepted, forwarding will not work.

    Openssh-portable can't handle UDS(unix domain socket) on Windows correctly (for now).

3. Even Openssh-portable handles UDS correctly, forwarding still can't work.

    > Support for Unix domain sockets was introduced in Windows 10 Insider Build 17063. It became generally
    available in version 1809 (aka the October 2018 Update), and in Windows Server 1809/2019.

    GnuPG on Windows has not utilized native UDS support yet. It simulates a UDS using a TCP stream socket with
    customized connect step. So without extra tools, you can't really connect openssh-portable to GnuPG.

[1]: https://docs.github.com/en/actions/managing-workflow-runs/downloading-workflow-artifacts
[2]: https://github.com/cqjjjzr/gpg-bridge/releases
