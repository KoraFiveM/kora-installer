import { register } from '../../index'

const messages = {
  'resources.resourcePage.couldResource': 'Could not {a} {resource}.',
  'resources.resourcePage.arrowleftNodeResourcesName': '{arrowLeft, node} Resources of {name}',
  'resources.resourcePage.pinnedReleaseSlice': ' · pinned to release {slice}',
  'resources.resourcePage.nodeNodeResourcesFolder': '{node, node} · resources/{folder}{value}',
  'resources.resourcePage.actions': 'Actions',
  'resources.resourcePage.whichVersionCode': 'Which version of the code',
  'resources.resourcePage.stopId': 'Stop {id}?',
  'resources.resourcePage.restartId': 'Restart {id}?',
  'resources.resourcePage.resourceStopsEveryoneServer': 'The resource stops for everyone on the server until it is started again. Other resources that need it may stop working. The server itself keeps running.',
  'resources.resourcePage.resourceStopsStartsAgain': 'The resource stops and starts again. Players may notice a short interruption in what it does. The server itself keeps running.',
  'resources.resourcePage.stopResource': 'Stop resource',
  'resources.resourcePage.restartResource': 'Restart resource',
} as const

register('en', messages)
export default messages
