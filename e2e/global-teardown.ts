import { stopBackend, cleanTestData } from './helpers/backend';
export default async function globalTeardown() {
  stopBackend();
  cleanTestData();
}
