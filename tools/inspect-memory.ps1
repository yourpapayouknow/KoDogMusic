#requires -Version 7.0
# 只读统计进程驻留页的映像、映射和私有来源。
[CmdletBinding()]
param([Parameter(Mandatory)][int]$TargetProcessId, [Parameter(Mandatory)][string]$OutputFile)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputFile) { throw 'Use a new report path.' }
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class KoDogPages {
    [StructLayout(LayoutKind.Sequential)] struct Region {
        public IntPtr Base, Allocation;
        public uint AllocationProtect;
        public ushort Partition;
        public UIntPtr Size;
        public uint State, Protect, Type;
    }
    public sealed class Row {
        public string Source, Kind;
        public long ResidentBytes, PrivateBytes, SharedBytes;
    }
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, int id);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError=true)] static extern UIntPtr VirtualQueryEx(IntPtr process, IntPtr address, out Region region, UIntPtr size);
    [DllImport("psapi.dll", SetLastError=true)] static extern bool QueryWorkingSet(IntPtr process, IntPtr buffer, uint size);
    [DllImport("psapi.dll", CharSet=CharSet.Unicode)] static extern uint GetMappedFileNameW(IntPtr process, IntPtr address, StringBuilder name, uint size);
    // 查询驻留页并按来源累加，不读取页内容。
    public static Row[] Prtn(int id) {
        if (IntPtr.Size != 8 || Marshal.SizeOf<Region>() != 48) throw new InvalidOperationException("Requires Windows x64.");
        var process = OpenProcess(0x410, false, id);
        if (process == IntPtr.Zero) throw new Win32Exception();
        const int capacity = 2 * 1024 * 1024;
        var buffer = Marshal.AllocHGlobal(capacity);
        try {
            if (!QueryWorkingSet(process, buffer, capacity)) throw new Win32Exception();
            var count = Marshal.ReadInt64(buffer);
            if (count < 0 || count > capacity / 8 - 1) throw new InvalidOperationException("Invalid page count.");
            var pages = new long[(int)count];
            Marshal.Copy(IntPtr.Add(buffer, 8), pages, 0, pages.Length);
            Array.Sort(pages);
            var rows = new Dictionary<string, Row>();
            var names = new Dictionary<long, string>();
            long end = -1;
            Region region = default;
            string source = "", kind = "";
            foreach (var flags in pages) {
                var address = flags & ~4095L;
                if (address >= end) {
                    if (VirtualQueryEx(process, new IntPtr(address), out region, (UIntPtr)48) == UIntPtr.Zero) throw new Win32Exception();
                    end = region.Base.ToInt64() + (long)region.Size.ToUInt64();
                    kind = region.Type == 0x1000000 ? "Image" : region.Type == 0x40000 ? "Mapped" : "Private";
                    var allocation = region.Allocation.ToInt64();
                    if (!names.TryGetValue(allocation, out source)) {
                        var name = new StringBuilder(32768);
                        source = region.Type != 0x20000 && GetMappedFileNameW(process, new IntPtr(address), name, (uint)name.Capacity) > 0 ? name.ToString() : kind;
                        names[allocation] = source;
                    }
                }
                var key = kind + "|" + source;
                if (!rows.TryGetValue(key, out var row)) rows[key] = row = new Row { Source=source, Kind=kind };
                row.ResidentBytes += 4096;
                if ((flags & 256) != 0) row.SharedBytes += 4096; else row.PrivateBytes += 4096;
            }
            var result = new Row[rows.Count]; rows.Values.CopyTo(result, 0); return result;
        } finally { Marshal.FreeHGlobal(buffer); CloseHandle(process); }
    }
}
'@
$taskRows = [KoDogPages]::Prtn($TargetProcessId) | Sort-Object ResidentBytes -Descending
$taskRows | Export-Csv -LiteralPath $OutputFile -NoTypeInformation -Encoding utf8
$taskRows | Select-Object -First 20 @{n='Source';e={Split-Path $_.Source -Leaf}},Kind,@{n='ResidentMB';e={[math]::Round($_.ResidentBytes/1e6,3)}},@{n='PrivateMB';e={[math]::Round($_.PrivateBytes/1e6,3)}},@{n='SharedMB';e={[math]::Round($_.SharedBytes/1e6,3)}} | Format-Table -AutoSize
