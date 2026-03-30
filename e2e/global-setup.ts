import { startBackend } from './helpers/backend';
export default async function globalSetup() {
  console.log('Starting backend...');
  await startBackend();
  console.log('Backend ready, seeded.');
}
