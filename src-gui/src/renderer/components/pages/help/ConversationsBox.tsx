import { useState, useEffect, useMemo } from "react";
import {
  Box,
  Typography,
  makeStyles,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  IconButton,
  TableContainer,
  Table,
  TableHead,
  TableRow,
  TableCell,
  TableBody,
  Paper,
  Badge,
  TextField,
  CircularProgress,
  InputAdornment,
  Tooltip,
  List,
  ListItem,
  ListItemIcon,
  Link,
} from "@material-ui/core";
import ChatIcon from '@material-ui/icons/Chat';
import SendIcon from '@material-ui/icons/Send';
import InfoBox from "renderer/components/modal/swap/InfoBox";
import TruncatedText from "renderer/components/other/TruncatedText";
import clsx from 'clsx';
import { useAppSelector, useAppDispatch, useUnreadMessagesCount } from "store/hooks";
import { markMessagesAsSeen } from "store/features/conversationsSlice";
import { appendFeedbackMessageViaHttp, fetchAllConversations } from "renderer/api";
import { useSnackbar } from "notistack";
import logger from "utils/logger";
import AttachmentIcon from '@material-ui/icons/Attachment';
import { PrimitiveDateTimeString } from "renderer/api";
import { Message } from "models/apiModel";

// Styles
const useStyles = makeStyles((theme) => ({
  content: { display: "flex", flexDirection: "column", alignItems: "flex-start", gap: theme.spacing(2) },
  tableContainer: { maxHeight: 300 },
  messagesContainer: { display: 'flex', flexDirection: 'column', gap: theme.spacing(1), maxHeight: 400, overflowY: 'auto', padding: theme.spacing(1) },
  messageRow: { display: 'flex', marginTop: theme.spacing(1) },
  staffRow: { justifyContent: 'flex-start' },
  userRow: { justifyContent: 'flex-end' },
  messageBubble: { padding: theme.spacing(1.5), borderRadius: theme.shape.borderRadius * 2, maxWidth: '75%', wordBreak: 'break-word', boxShadow: theme.shadows[1] },
  staffBubble: { border: `1px solid ${theme.palette.divider}`, color: theme.palette.text.primary, borderRadius: theme.spacing(2) },
  userBubble: { backgroundColor: theme.palette.primary.main, color: theme.palette.primary.contrastText, borderRadius: theme.spacing(2) },
  timestamp: { marginTop: theme.spacing(0.5), fontSize: '0.75rem', opacity: 0.7, textAlign: 'right' },
  inputArea: { marginTop: theme.spacing(2) },
  attachmentList: {
    marginTop: theme.spacing(1),
    marginBottom: theme.spacing(1),
    paddingLeft: theme.spacing(2),
  },
  attachmentItem: {
    paddingTop: theme.spacing(0.5),
    paddingBottom: theme.spacing(0.5),
  }
}));

// Utilities

// Updated function to parse 9-element tuple and format it
function formatDateTime(dateTime: PrimitiveDateTimeString | null | undefined): string {
  if (!dateTime || !Array.isArray(dateTime) || dateTime.length !== 9) {
    // Basic validation for null, undefined, or incorrect structure
    return "Invalid Date Input";
  }
  
  try {
    const [year, dayOfYear, hour, minute, second, nanoseconds, offsetH, offsetM, offsetS] = dateTime;

    // More robust validation (example)
    if (year < 1970 || dayOfYear < 1 || dayOfYear > 366 || hour < 0 || hour > 23 || minute < 0 || minute > 59 || second < 0 || second > 59 || nanoseconds < 0 || nanoseconds > 999999999) {
        logger.warn("Invalid date components detected in formatDateTime tuple", { dateTime });
        return "Invalid Date Components";
    }
    
    // Calculate total offset in seconds (handle potential non-zero offsets)
    const totalOffsetSeconds = (offsetH * 3600) + (offsetM * 60) + offsetS;

    // Calculate milliseconds from nanoseconds
    const milliseconds = Math.floor(nanoseconds / 1_000_000);

    // Create Date object for the start of the year *in UTC*
    const date = new Date(Date.UTC(year, 0, 1)); // Month is 0-indexed (January)
    
    // Add (dayOfYear - 1) days to get the correct date *in UTC*
    date.setUTCDate(date.getUTCDate() + dayOfYear - 1);

    // Set the time components *in UTC*
    date.setUTCHours(hour);
    date.setUTCMinutes(minute);
    date.setUTCSeconds(second);
    date.setUTCMilliseconds(milliseconds);

    // Adjust for the timezone offset to get the correct UTC time
    // Subtract the offset because Date.UTC assumes UTC, but the components might be for a different offset
    date.setTime(date.getTime() - totalOffsetSeconds * 1000);

    // Final validation
    if (isNaN(date.getTime())) {
        logger.warn("Calculated date is invalid in formatDateTime", { dateTime });
        return "Invalid Calculated Date";
    }

    // Format to a readable string (e.g., "YYYY-MM-DD HH:MM:SS UTC")
    const yyyy = date.getUTCFullYear();
    const mm = String(date.getUTCMonth() + 1).padStart(2, '0');
    const dd = String(date.getUTCDate()).padStart(2, '0');
    const HH = String(date.getUTCHours()).padStart(2, '0');
    const MM = String(date.getUTCMinutes()).padStart(2, '0');
    const SS = String(date.getUTCSeconds()).padStart(2, '0');

    return `${yyyy}-${mm}-${dd} ${HH}:${MM}:${SS} UTC`;

  } catch (e) {
    logger.error(e, "Error formatting datetime tuple", { dateTime });
    return "Invalid Date Format";
  }
}

// Hook: sorted feedback IDs by latest activity, then unread
function useSortedFeedbackIds() {
  const ids = useAppSelector((s) => s.conversations.knownFeedbackIds || []);
  const conv = useAppSelector((s) => s.conversations.conversations);
  const seen = useAppSelector((s) => new Set(s.conversations.seenMessages));
  return useMemo(() => {
    const arr = ids.map((id) => {
      const msgs = conv[id] || [];
      const unread = msgs.filter((m) => m.is_from_staff && !seen.has(m.id.toString())).length;
      const latest = msgs.reduce((d, m) => {
        try {
          const formattedDate = formatDateTime(m.created_at);
          if (formattedDate.startsWith("Invalid")) return d;
          const t = new Date(formattedDate).getTime();
          return isNaN(t) ? d : Math.max(d, t);
        } catch(e) { return d; }
      }, 0);
      return { id, unread, latest };
    });
    arr.sort((a, b) => b.latest - a.latest || (b.unread > 0 ? 1 : 0) - (a.unread > 0 ? 1 : 0));
    return arr.map((x) => x.id);
  }, [ids, conv, seen]);
}

// Main component
export default function ConversationsBox() {
  const classes = useStyles();
  const sortedIds = useSortedFeedbackIds();
  const [openId, setOpenId] = useState<string | null>(null);

  useEffect(() => {
    // Fetch conversations via API function (handles its own dispatch)
    fetchAllConversations();
  }, []);

  return (
    <InfoBox
      title="Developer Responses"
      icon={null}
      loading={false}
      mainContent={
        <Box className={classes.content}>
          <Typography variant="subtitle2">
            View your past feedback submissions and any replies from the development team.
          </Typography>
          {sortedIds.length === 0 ? (
            <Typography variant="body2">No feedback submitted yet.</Typography>
          ) : (
            <TableContainer component={Paper} className={classes.tableContainer}>
              <Table stickyHeader size="small">
                <TableHead>
                  <TableRow>
                    <TableCell style={{ width: '25%' }}>Last Message</TableCell>
                    <TableCell style={{ width: '60%' }}>Preview</TableCell>
                    <TableCell align="right" style={{ width: '15%' }} />
                  </TableRow>
                </TableHead>
                <TableBody>
                  {sortedIds.map((id) => (
                    <ConversationRow key={id} feedbackId={id} onOpen={setOpenId} />
                  ))}
                </TableBody>
              </Table>
            </TableContainer>
          )}
        </Box>
      }
      additionalContent={
        openId && (
          <ConversationModal
            open={!!openId}
            onClose={() => setOpenId(null)}
            feedbackId={openId}
          />
        )
      }
    />
  );
}

// Single row
function ConversationRow({ feedbackId, onOpen }: { feedbackId: string, onOpen: (id: string) => void }) {
  const classes = useStyles();
  const msgs = useAppSelector((s) => s.conversations.conversations[feedbackId] || []);
  const unread = useUnreadMessagesCount(feedbackId);
  const sorted = useMemo(
    () =>
      [...msgs].sort((a, b) => {
        try {
          const formattedDateA = formatDateTime(a.created_at);
          const formattedDateB = formatDateTime(b.created_at);
          if (formattedDateA.startsWith("Invalid")) return 1;
          if (formattedDateB.startsWith("Invalid")) return -1;
          const dateA = new Date(formattedDateA).getTime();
          const dateB = new Date(formattedDateB).getTime();
          if (isNaN(dateA)) return 1;
          if (isNaN(dateB)) return -1;
          return dateB - dateA;
        } catch (e) { return 0; }
      }),
    [msgs]
  );
  const lastMsg = sorted[0];
  const time = lastMsg ? formatDateTime(lastMsg.created_at) : '-';
  const content = lastMsg ? lastMsg.content : 'No messages yet';
  const preview = (() => {
    return content;
  })();
  const hasStaff = useMemo(() => msgs.some((m) => m.is_from_staff), [msgs]);

  return (
    <TableRow>
      <TableCell style={{ width: '25%' }}>{time}</TableCell>
      <TableCell style={{ width: '60%' }}>
        "<TruncatedText limit={30}>{preview}</TruncatedText>"
      </TableCell>
      <TableCell align="right" style={{ width: '15%' }}>
        <Badge badgeContent={unread} color="primary" overlap="rectangular">
          <Tooltip title={hasStaff ? 'Open Conversation' : 'No developer has responded'} arrow>
            <span>
              <IconButton size="small" onClick={() => onOpen(feedbackId)} disabled={!hasStaff}>
                <ChatIcon />
              </IconButton>
            </span>
          </Tooltip>
        </Badge>
      </TableCell>
    </TableRow>
  );
}

// Modal
function ConversationModal({ open, onClose, feedbackId }: { open: boolean, onClose: () => void, feedbackId: string }) {
  const classes = useStyles();
  const dispatch = useAppDispatch();
  const { enqueueSnackbar } = useSnackbar();
  const msgs = useAppSelector((s): Message[] => s.conversations.conversations[feedbackId] || []);
  const seen = useAppSelector((s) => new Set(s.conversations.seenMessages));
  const [text, setText] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (open) {
      const unseen = msgs.filter((m) => !seen.has(m.id.toString()));
      if (unseen.length) dispatch(markMessagesAsSeen(unseen));
    }
  }, [open, msgs, seen, dispatch]);

  const sorted = useMemo(
    () =>
      [...msgs].sort((a, b) => {
        try {
          const formattedDateA = formatDateTime(a.created_at);
          const formattedDateB = formatDateTime(b.created_at);
          if (formattedDateA.startsWith("Invalid")) return 1;
          if (formattedDateB.startsWith("Invalid")) return -1;
          const dateA = new Date(formattedDateA).getTime();
          const dateB = new Date(formattedDateB).getTime();
          if (isNaN(dateA)) return 1;
          if (isNaN(dateB)) return -1;
          return dateB - dateA;
        } catch(e) { return 0; }
      }),
    [msgs]
  );

  const sendMessage = async () => {
    if (!text.trim()) return;
    setLoading(true);
    try {
      await appendFeedbackMessageViaHttp(feedbackId, text);
      setText('');
      enqueueSnackbar('Message sent successfully!', { variant: 'success' });
      fetchAllConversations();
    } catch (e) {
      logger.error(e, 'Send failed');
      enqueueSnackbar('Failed to send message. Please try again.', { variant: 'error' });
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth scroll="paper">
      <DialogTitle>Conversation <TruncatedText limit={8} truncateMiddle>{feedbackId}</TruncatedText></DialogTitle>
      <DialogContent dividers style={{ display: 'flex', flexDirection: 'column' }}>
        {sorted.length === 0 ? (
          <Typography variant="body2">No messages in this conversation.</Typography>
        ) : (
          <Box className={classes.messagesContainer} style={{ flexGrow: 1, overflowY: 'auto' }}>
            {sorted.map((m: Message) => {
              const raw = m.content;
              return (
                <Box
                  key={m.id}
                  className={clsx(
                    classes.messageRow,
                    m.is_from_staff ? classes.staffRow : classes.userRow
                  )}
                >
                  <Box className={clsx(
                    classes.messageBubble,
                    m.is_from_staff ? classes.staffBubble : classes.userBubble
                  )}
                  >
                    <Typography variant="body1" style={{ whiteSpace: 'pre-wrap' }}>
                      {raw}
                    </Typography>
                    {m.attachments && m.attachments.length > 0 && (
                      <List dense disablePadding className={classes.attachmentList}>
                        {m.attachments.map((att) => (
                          <ListItem key={att.id} className={classes.attachmentItem}>
                            <ListItemIcon style={{ minWidth: 'auto', marginRight: '8px' }}>
                              <AttachmentIcon fontSize="small" />
                            </ListItemIcon>
                            <Link 
                              href="#" 
                              onClick={(e) => {
                                e.preventDefault(); 
                                alert(`Attachment Key: ${att.key}\n\nContent:\n${att.content}`);
                              }}
                              variant="body2"
                              color="inherit"
                              underline="none"
                            >
                              {att.key}
                            </Link>
                          </ListItem>
                        ))}
                      </List>
                    )}
                    <Typography variant="caption" className={classes.timestamp}>
                      {m.is_from_staff ? 'Developer' : 'You'} · {formatDateTime(m.created_at)}
                    </Typography>
                  </Box>
                </Box>
              );
            })}
          </Box>
        )}
        <Box className={classes.inputArea} style={{ flexShrink: 0 }}>
          <TextField
            variant="outlined"
            fullWidth
            placeholder="Type your message..."
            value={text}
            onChange={(e) => setText(e.target.value)}
            disabled={loading}
            InputProps={{
              endAdornment: (
                <InputAdornment position="end">
                  <IconButton
                    color="primary"
                    onClick={sendMessage}
                    disabled={!text.trim() || loading}
                  >
                    {loading ? <CircularProgress size={24} /> : <SendIcon />}
                  </IconButton>
                </InputAdornment>
              ),
            }}
          />
        </Box>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} variant="outlined">
          Close
        </Button>
      </DialogActions>
    </Dialog>
  );
}
