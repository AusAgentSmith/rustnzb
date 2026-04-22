import * as path from 'path';
import * as fs from 'fs';
import { stopAllBackends, cleanAllBackendData } from './helpers/backend';

export default async function globalTeardown() {
  stopAllBackends();
  cleanAllBackendData();
  const stateFile = path.join(__dirname, 'auth-state.json');
  if (fs.existsSync(stateFile)) fs.unlinkSync(stateFile);
}
