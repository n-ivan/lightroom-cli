return {
  LrSdkVersion = 14.0,
  LrSdkMinimumVersion = 12.0,
  LrToolkitIdentifier = 'dev.lrcli.bridge',
  LrPluginName = 'lrcli',
  LrInitPlugin = 'Init.lua',
  LrPluginInfoProvider = 'PluginInfoProvider.lua',
  LrForceInitPlugin = true,
  -- LrForceInitPlugin only takes effect through a menu item, and the Library
  -- menu doesn't exist when Lightroom opens in another module; File does.
  LrExportMenuItems = {
    { title = 'lrcli Status…', file = 'StatusMenuItem.lua' },
  },
  VERSION = { major = 0, minor = 1, revision = 0 },
}
