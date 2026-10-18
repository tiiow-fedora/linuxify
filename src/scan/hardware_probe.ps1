# Linuxify hardware probe. READ-ONLY: only Get-* / CIM queries, no changes.
# Emits one JSON object on stdout. Every section is wrapped in try/catch so a
# failure (e.g. a cmdlet that needs admin) becomes an "error" string instead of
# killing the whole probe.
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$out = [ordered]@{}
$errors = [ordered]@{}

function Try-Section($name, [scriptblock]$block) {
    try { $script:out[$name] = & $block }
    catch { $script:errors[$name] = $_.Exception.Message; $script:out[$name] = $null }
}

$out.is_admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

Try-Section 'os' {
    $o = Get-CimInstance Win32_OperatingSystem
    [ordered]@{ caption = $o.Caption; version = $o.Version; build = $o.BuildNumber; arch = $o.OSArchitecture }
}
Try-Section 'cpu' {
    @(Get-CimInstance Win32_Processor | ForEach-Object {
        [ordered]@{ name = $_.Name.Trim(); manufacturer = $_.Manufacturer; cores = $_.NumberOfCores; threads = $_.NumberOfLogicalProcessors }
    })
}
Try-Section 'ram_bytes' { [uint64](Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory }
Try-Section 'computer' {
    $c = Get-CimInstance Win32_ComputerSystem
    [ordered]@{ manufacturer = $c.Manufacturer; model = $c.Model }
}
Try-Section 'gpus' {
    @(Get-CimInstance Win32_VideoController | ForEach-Object {
        [ordered]@{ name = $_.Name; pnp_id = $_.PNPDeviceID; driver_version = $_.DriverVersion }
    })
}
Try-Section 'disks' {
    @(Get-Disk | ForEach-Object {
        $d = $_
        [ordered]@{
            number = $d.Number; model = $d.FriendlyName; size = [uint64]$d.Size
            bus_type = [string]$d.BusType; partition_style = [string]$d.PartitionStyle
            is_boot = $d.IsBoot; is_system = $d.IsSystem
            unallocated = [uint64]($d.Size - $d.AllocatedSize)
            partitions = @(Get-Partition -DiskNumber $d.Number -ErrorAction SilentlyContinue | ForEach-Object {
                $p = $_
                $v = $null
                try { $v = $p | Get-Volume -ErrorAction Stop } catch {}
                [ordered]@{
                    number = $p.PartitionNumber; drive_letter = [string]$p.DriveLetter
                    size = [uint64]$p.Size; offset = [uint64]$p.Offset
                    gpt_type = [string]$p.GptType; mbr_type = $p.MbrType
                    is_boot = $p.IsBoot; is_system = $p.IsSystem; is_hidden = $p.IsHidden
                    file_system = if ($v) { [string]$v.FileSystem } else { $null }
                    label = if ($v) { [string]$v.FileSystemLabel } else { $null }
                    free = if ($v) { [uint64]$v.SizeRemaining } else { $null }
                }
            })
        }
    })
}
Try-Section 'storage_controllers' {
    @(Get-CimInstance Win32_PnPEntity -Filter "PNPClass='SCSIAdapter' OR PNPClass='HDC' OR PNPClass='System'" |
        Where-Object { $_.Name -match 'RAID|RST|VMD|Volume Management|Rapid Storage|NVM|SATA|AHCI|Storage' } |
        ForEach-Object { [ordered]@{ name = $_.Name; pnp_class = $_.PNPClass; pnp_id = $_.PNPDeviceID; service = $_.Service } })
}
Try-Section 'net_adapters' {
    @(Get-NetAdapter -Physical | ForEach-Object {
        [ordered]@{ name = $_.Name; description = $_.InterfaceDescription; media = [string]$_.NdisPhysicalMedium; pnp_id = $_.PnPDeviceID; status = [string]$_.Status }
    })
}
Try-Section 'c_drive' {
    $v = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='C:'"
    [ordered]@{ size = [uint64]$v.Size; free = [uint64]$v.FreeSpace; file_system = $v.FileSystem }
}
# Exact shrinkable size needs admin on most systems; it is optional.
Try-Section 'c_shrink' {
    $s = Get-PartitionSupportedSize -DriveLetter C
    $p = Get-Partition -DriveLetter C
    [ordered]@{ current = [uint64]$p.Size; min = [uint64]$s.SizeMin }
}
# BitLocker: the cmdlet needs admin. The Shell property works without it.
Try-Section 'bitlocker_cmdlet' {
    @(Get-BitLockerVolume | ForEach-Object {
        [ordered]@{ mount = $_.MountPoint; protection = [string]$_.ProtectionStatus; status = [string]$_.VolumeStatus; percent = $_.EncryptionPercentage }
    })
}
Try-Section 'bitlocker_shell' {
    $shell = New-Object -ComObject Shell.Application
    @(Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | ForEach-Object {
        $raw = $shell.NameSpace($_.DeviceID + '\').Self.ExtendedProperty('System.Volume.BitLockerProtection')
        [ordered]@{ mount = $_.DeviceID; value = $raw }
    })
}

$out.errors = $errors
ConvertTo-Json -InputObject $out -Depth 6 -Compress
