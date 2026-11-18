import { register } from '../../index'

const messages = {
  'servers.joinHelp.title': 'Players cannot join my server',
  'servers.joinHelp.intro': 'Work through these in order and stop when it works. Each step fixes one thing that keeps players out. The "Can players reach this server?" card on the server page checks most of this for you.',
  'servers.joinHelp.back': 'Back to the servers',
  'servers.joinHelp.portKnown': 'Your game port is {port}.',
  'servers.joinHelp.portUnknown': 'The game port is 30120 unless you chose another one. Open this page from a server to see its own port.',
  'servers.joinHelp.runningTitle': '1. Is the server running and listening?',
  'servers.joinHelp.runningBody': 'The server has to be started and listening on its game port (TCP and UDP). If its status is not "Running", start it from the server page. If another program already uses the port, change the port in the server settings and check again.',
  'servers.joinHelp.thisComputerTitle': '2. Try it from this computer first',
  'servers.joinHelp.thisComputerBody': 'In FiveM press F8 and type {command, node}. If that fails, the problem is the server itself, not your network: read the server console for errors.',
  'servers.joinHelp.homeNetworkTitle': '3. Try it from another computer on your home network',
  'servers.joinHelp.homeNetworkBody': "Connect to this computer's address on your own network, which the server page shows (it looks like {address, node}). If that works but friends on the internet cannot join, the cause is the firewall or your router: carry on.",
  'servers.joinHelp.firewallTitle': '4. Let the game port through the firewall',
  'servers.joinHelp.firewallBody': 'Allow TCP and UDP port {port} in. Kora does this when it installs a server, or press "Open port in the firewall" on the card. Windows may have asked when the server first started: if that window was closed with Cancel, the rule is missing. On Linux with ufw: {command, node}.',
  'servers.joinHelp.routerTitle': '5. Forward the port on your router',
  'servers.joinHelp.routerIntro': 'Your router is the front door of your home network. Players on the internet only get in if it sends port {port} to this computer.',
  'servers.joinHelp.routerStep1': 'Open your router\'s settings page (often {first, node} or {second, node}; the address and password are usually on a sticker on the router).',
  'servers.joinHelp.routerStep2': 'Find "Port forwarding" (sometimes "Virtual server", "NAT" or "Applications").',
  'servers.joinHelp.routerStep3': 'Add a rule: protocol TCP and UDP (both; if there is only one choice, make two rules), outside and inside port {port}, destination this computer\'s address on your network.',
  'servers.joinHelp.routerStep4': 'Save, then press "Check again" on the server page.',
  'servers.joinHelp.routerStep5': 'Tip: give this computer a fixed address in the router ("DHCP reservation"), or the rule stops working when its address changes.',
  'servers.joinHelp.providerTitle': '6. Your internet provider may not allow it',
  'servers.joinHelp.providerBody': 'Some providers (often mobile, satellite and some fibre connections) share one public address between many customers (CGNAT), so port forwarding cannot work. A sign: the address your router shows for the internet is different from the one a "what is my IP" website shows, or it starts with 100.64 to 100.127. Ask the provider for a public address, host the server somewhere that has one, or use Tailscale to play with a private group of friends.',
  'servers.joinHelp.outsideTitle': '7. Test from outside your home',
  'servers.joinHelp.outsideBody': 'Ask a friend who is not on your network to connect to your public address and port {port}. Do not test with your own public address from inside your home: many routers cannot loop back to themselves, so it fails even when everything is right.',
  'servers.joinHelp.stuckTitle': 'Still stuck?',
  'servers.joinHelp.stuckBody': 'Make a support bundle from the server page (it leaves out passwords and keys) and attach it when you ask for help.',
  'servers.joinHelp.link': 'Players still cannot join? Read the troubleshooting guide',
} as const

register('en', messages)
export default messages
