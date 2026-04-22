import * as path from 'path';
import * as fs from 'fs';

const AUTH_STATE = path.resolve(__dirname, '../auth-state.json');

/** Read the access token from auth-state.json (written by global-setup). */
export function readToken(): string {
  const state = JSON.parse(fs.readFileSync(AUTH_STATE, 'utf8'));
  const token = state.origins?.[0]?.localStorage?.find(
    (e: { name: string }) => e.name === 'access_token',
  )?.value;
  if (!token) throw new Error('No access_token found in auth-state.json');
  return token;
}
