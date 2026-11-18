//! Small Win32 process helpers: exe path of a PID, listening TCP ports of a PID,
//! PIDs by exe name.

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Full image path of a process, if we are allowed to ask.
pub fn exe_path(pid: u32) -> Option<String> {
    unsafe {
        let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Lower-case file name of a process's exe, e.g. "brave.exe".
pub fn exe_name(pid: u32) -> Option<String> {
    let p = exe_path(pid)?;
    Some(p.rsplit('\\').next().unwrap_or(&p).to_ascii_lowercase())
}

/// PIDs of all processes whose exe file name equals `name` (case-insensitive).
pub fn pids_by_name(name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let exe = String::from_utf16_lossy(&e.szExeFile[..n]);
                if exe.eq_ignore_ascii_case(name) {
                    out.push(e.th32ProcessID);
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// IPv4 TCP ports that `pid` is listening on.
pub fn listening_ports(pid: u32) -> Vec<u16> {
    let mut ports = Vec::new();
    unsafe {
        let mut size = 0u32;
        let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0);
        if size == 0 {
            return ports;
        }
        // u32-aligned buffer
        let mut buf = vec![0u32; (size as usize).div_ceil(4) + 16];
        size = (buf.len() * 4) as u32;
        let rc = GetExtendedTcpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if rc != 0 {
            return ports;
        }
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        for r in rows {
            if r.dwOwningPid == pid {
                let port = u16::from_be((r.dwLocalPort & 0xFFFF) as u16);
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
    }
    ports
}
