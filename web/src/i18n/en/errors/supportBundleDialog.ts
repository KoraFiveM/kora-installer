import { register } from '../../index'

const messages = {
  'errors.supportBundleDialog.createSupportBundle': 'Create a support bundle',
  'errors.supportBundleDialog.oneFileHandWhoever': 'One file you can hand to whoever is helping you, with your secrets removed.',
  'errors.supportBundleDialog.accountRoleCannotMake': 'Your account ({role}) cannot make a support bundle. Ask an administrator.',
  'errors.supportBundleDialog.koraNeverUploadsFile': 'Kora never uploads this file anywhere. It is saved on this computer, and it leaves only if you send it yourself. Licence keys, database logins, txAdmin accounts and tokens, webhook addresses, Git tokens and passwords are removed first.',
  'errors.supportBundleDialog.koraCollectItsVersion': "Kora will collect its version, your system, the System check, tool versions, your servers' settings (without secrets), recent jobs and problems, the activity log, install and update logs, the service and firewall state, and disk space. You see everything before it is saved.",
  'errors.supportBundleDialog.prepareShowMeWhat': 'Prepare it and show me what is in it',
  'errors.supportBundleDialog.preparing': 'Preparing…',
  'errors.supportBundleDialog.whatFileHoldSize': 'This is what the file will hold ({size}). Click a file to read it exactly as it will be saved.',
  'errors.supportBundleDialog.filesSupportBundle': 'Files in the support bundle',
  'errors.supportBundleDialog.redactionsHiddenJoin': '{redactions} hidden: {join}',
  'errors.supportBundleDialog.leftOutAltogetherBecause': 'Left out altogether, because they hold secrets',
  'errors.supportBundleDialog.spanNodeWhy': '{span, node}: it {why}',
  'errors.supportBundleDialog.textOpen': 'Text of {open}',
  'errors.supportBundleDialog.empty': '(empty)',
  'errors.supportBundleDialog.neverUploadedSavedSpan': '{never_uploaded} It will be saved in {span, node}.',
  'errors.supportBundleDialog.saveBundle': 'Save this bundle',
  'errors.supportBundleDialog.cancelSaveNothing': 'Cancel, save nothing',
  'errors.supportBundleDialog.saved': 'Saved',
  'errors.supportBundleDialog.sendOneFileWhoever': 'Send this one file to whoever is helping you.',
  'errors.supportBundleDialog.whereSaved': 'Where it was saved',
  'errors.supportBundleDialog.downloadNodeDownload': '{download, node} Download it',
  'errors.supportBundleDialog.close': 'Close',
} as const

register('en', messages)
export default messages
