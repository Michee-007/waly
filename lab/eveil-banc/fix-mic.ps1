# Lit et corrige mute/volume du micro par defaut (Core Audio, user-level).
# ASCII pur (piege PS 5.1).
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
class MMDeviceEnumeratorComObject { }
[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDeviceEnumerator {
    int NotImpl1();
    [PreserveSig] int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice device);
}
[Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDevice {
    [PreserveSig] int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
}
[Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IAudioEndpointVolume {
    int NotImpl1(); int NotImpl2();
    [PreserveSig] int GetChannelCount(out uint count);
    [PreserveSig] int SetMasterVolumeLevel(float level, ref Guid ctx);
    [PreserveSig] int SetMasterVolumeLevelScalar(float level, ref Guid ctx);
    [PreserveSig] int GetMasterVolumeLevel(out float level);
    [PreserveSig] int GetMasterVolumeLevelScalar(out float level);
    int NotImpl3(); int NotImpl4(); int NotImpl5(); int NotImpl6();
    [PreserveSig] int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, ref Guid ctx);
    [PreserveSig] int GetMute([MarshalAs(UnmanagedType.Bool)] out bool mute);
}
public static class MicFix {
    public static string Run() {
        var enumerator = (IMMDeviceEnumerator)(new MMDeviceEnumeratorComObject());
        IMMDevice dev;
        int hr = enumerator.GetDefaultAudioEndpoint(1 /*eCapture*/, 0 /*eConsole*/, out dev);
        if (hr != 0) return "GetDefaultAudioEndpoint: 0x" + hr.ToString("X");
        var iid = new Guid("5CDF2C82-841E-4546-9722-0CF74078229A");
        object o;
        hr = dev.Activate(ref iid, 1 /*CLSCTX_INPROC_SERVER*/, IntPtr.Zero, out o);
        if (hr != 0) return "Activate: 0x" + hr.ToString("X");
        var vol = (IAudioEndpointVolume)o;
        bool mute; float level;
        vol.GetMute(out mute);
        vol.GetMasterVolumeLevelScalar(out level);
        string avant = "avant: mute=" + mute + " volume=" + (int)(level * 100) + "%";
        var ctx = Guid.Empty;
        vol.SetMute(false, ref ctx);
        if (level < 0.5f) vol.SetMasterVolumeLevelScalar(0.8f, ref ctx);
        vol.GetMute(out mute);
        vol.GetMasterVolumeLevelScalar(out level);
        return avant + " | apres: mute=" + mute + " volume=" + (int)(level * 100) + "%";
    }
}
"@
[MicFix]::Run()
