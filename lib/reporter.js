import pc from 'picocolors';

const SECTION_WIDTH = 60;

export function formatSection(name, content) {
  const header = pc.bold(pc.cyan(`[${name}]`));
  const body = typeof content === 'string' ? content : JSON.stringify(content, null, 2);
  return `\n${header}\n${body}`;
}

export function formatReport(report) {
  const sections = [];

  sections.push(formatSection('SUMMARY', report.summary));

  sections.push(formatSection('CLASSIFICATION', [
    `  label:      ${pc.yellow(report.classification.label)}`,
    `  confidence: ${report.classification.confidence}`,
  ].join('\n')));

  const evidenceLines = report.evidence.length > 0
    ? report.evidence.map(e => `  ${e}`).join('\n')
    : '  (no specific evidence lines captured)';
  sections.push(formatSection('EVIDENCE', evidenceLines));

  const patchContent = report.patch
    ? colorDiff(report.patch)
    : '  (no patch — see NEXT for manual steps)';
  sections.push(formatSection('PATCH', patchContent));

  const verResult = report.verification.result === 'PASS'
    ? pc.green('PASS')
    : pc.red('FAIL');
  sections.push(formatSection('VERIFICATION', [
    `  Command:   ${report.verification.command}`,
    `  Exit code: ${report.verification.exit_code}`,
    `  Result:    ${verResult}`,
    report.verification.excerpt ? `\n  ${report.verification.excerpt}` : '',
  ].join('\n')));

  sections.push(formatSection('RISK', [
    `  Blast radius: ${report.risk.blast_radius}`,
    `  Rollback:     ${pc.dim(report.risk.rollback_hint)}`,
  ].join('\n')));

  sections.push(formatSection('NEXT', `  ${report.next}`));

  const divider = pc.dim('─'.repeat(SECTION_WIDTH));
  return `\n${divider}${sections.join('\n')}\n${divider}\n`;
}

function colorDiff(patch) {
  return patch.split('\n').map(line => {
    if (line.startsWith('+++') || line.startsWith('---')) return pc.bold(line);
    if (line.startsWith('+')) return pc.green(line);
    if (line.startsWith('-')) return pc.red(line);
    if (line.startsWith('@@')) return pc.cyan(line);
    return line;
  }).join('\n');
}
